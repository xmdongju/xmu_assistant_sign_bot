use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fmt::Debug,
    sync::{Arc, LazyLock},
};

const EMBEDDED_CATALOG: &str = include_str!("../../../app_data/locations.json");
pub static LOCATIONS: LazyLock<LocationService> = LazyLock::new(LocationService::load);

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Region {
    XiangAn,
    SiMing,
    ZhangZhou,
}

impl Debug for Region {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Region::XiangAn => write!(f, "\"翔安校区\""),
            Region::SiMing => write!(f, "\"思明校区\""),
            Region::ZhangZhou => write!(f, "\"漳州校区\""),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub region: Region,
    pub name: &'static str,
    pub longitude: f32,
    pub latitude: f32,
}

impl PartialEq for Location {
    fn eq(&self, other: &Self) -> bool {
        self.region == other.region && self.name == other.name
    }
}

impl Eq for Location {}

impl Location {
    pub fn new(region: Region, name: &'static str, longitude: f32, latitude: f32) -> Self {
        Self {
            region,
            name,
            longitude,
            latitude,
        }
    }
}

pub struct LocationService {
    pub locations: Vec<Arc<Location>>,
    aliases: Vec<Vec<&'static str>>,
    pub external_file_loaded: bool,
}

#[derive(Deserialize)]
struct Catalog {
    schema_version: u32,
    coordinate_system: String,
    locations: Vec<CatalogEntry>,
}

#[derive(Deserialize)]
struct CatalogEntry {
    region: Region,
    name: String,
    #[serde(default)]
    aliases: Vec<String>,
    longitude: f32,
    latitude: f32,
}

impl LocationService {
    /// 保留原有固定位置列表构造接口。
    pub fn new<const N: usize>(locations: [Location; N]) -> Self {
        Self {
            locations: locations.into_iter().map(Arc::new).collect(),
            aliases: vec![Vec::new(); N],
            external_file_loaded: false,
        }
    }

    fn load() -> Self {
        if let Some(path) = std::env::var("XMU_LOCATION_FILE")
            .ok()
            .filter(|s| !s.trim().is_empty())
        {
            match std::fs::read_to_string(path)
                .ok()
                .and_then(|text| Self::from_json(&text).ok())
            {
                Some(mut service) => {
                    service.external_file_loaded = true;
                    return service;
                }
                None => tracing::warn!("位置文件无效或无法读取，使用内置位置库"),
            }
        }
        Self::from_json(EMBEDDED_CATALOG).expect("built-in campus locations must be valid")
    }

    pub fn from_json(text: &str) -> anyhow::Result<Self> {
        if text.len() > 1024 * 1024 {
            anyhow::bail!("位置文件过大");
        }
        let catalog: Catalog = serde_json::from_str(text)?;
        if catalog.schema_version != 1 || catalog.coordinate_system != "xmu-map-native" {
            anyhow::bail!("位置库版本或坐标格式不支持");
        }
        if catalog.locations.is_empty() || catalog.locations.len() > 2048 {
            anyhow::bail!("位置数量不合法");
        }
        let mut names = HashSet::new();
        for row in &catalog.locations {
            let valid_text = |s: &str| {
                let n = s.trim().chars().count();
                (2..=128).contains(&n)
            };
            let valid_coords = row.longitude.is_finite()
                && row.latitude.is_finite()
                && (117.9..=118.5).contains(&row.longitude)
                && (24.25..=24.8).contains(&row.latitude);
            if !valid_text(&row.name)
                || !valid_coords
                || row.aliases.len() > 32
                || row.aliases.iter().any(|s| !valid_text(s))
                || !names.insert(row.name.clone())
            {
                anyhow::bail!("位置名称、别名或坐标不合法");
            }
        }
        // LazyLock loads once. Validate first, then retain these bounded names for existing
        // Location/LocationStore APIs that use static strings and persist names as text.
        let mut locations = Vec::with_capacity(catalog.locations.len());
        let mut aliases = Vec::with_capacity(catalog.locations.len());
        for row in catalog.locations {
            let name: &'static str = Box::leak(row.name.into_boxed_str());
            aliases.push(
                row.aliases
                    .into_iter()
                    .map(|s| Box::leak(s.into_boxed_str()) as &'static str)
                    .collect(),
            );
            locations.push(Arc::new(Location::new(
                row.region,
                name,
                row.longitude,
                row.latitude,
            )));
        }
        Ok(Self {
            locations,
            aliases,
            external_file_loaded: false,
        })
    }

    pub fn query(&self, fullname: &str) -> Option<Location> {
        let campus = if fullname.contains("翔安") {
            Some(Region::XiangAn)
        } else if fullname.contains("漳州") {
            Some(Region::ZhangZhou)
        } else if fullname.contains("思明") || fullname.contains("海韵") {
            Some(Region::SiMing)
        } else {
            None
        };
        let mut best: Option<usize> = None;
        let mut best_score = 0;
        let mut ambiguous = false;
        for (index, loc) in self.locations.iter().enumerate() {
            if campus.as_ref().is_some_and(|hint| hint != &loc.region) {
                continue;
            }
            let score = std::iter::once(&loc.name)
                .chain(self.aliases[index].iter())
                .filter(|name| contains_name(fullname, name))
                .map(|name| name.chars().count())
                .max()
                .unwrap_or(0);
            if score > best_score {
                best = Some(index);
                best_score = score;
                ambiguous = false;
            } else if score > 0 && score == best_score {
                ambiguous = true;
            }
        }
        if ambiguous {
            None
        } else {
            best.map(|index| self.locations[index].as_ref().clone())
        }
    }

    pub fn get(&self, id: usize) -> Option<Arc<Location>> {
        self.locations.get(id).cloned()
    }

    /// 找到距离小于等于 distance_meter 米的位置
    pub fn find(
        &self,
        latitude: f32,
        longitude: f32,
        distance_meter: f64,
    ) -> Option<Arc<Location>> {
        let local = geoutils::Location::new(latitude as f64, longitude as f64);
        let mut ret = None;
        let mut min_dis = f64::MAX;
        for loc in &self.locations {
            let check_loc = geoutils::Location::new(loc.latitude as f64, loc.longitude as f64);
            if let Ok(dis) = local.distance_to(&check_loc)
                && dis.meters() < min_dis
            {
                min_dis = dis.meters();
                ret = Some(loc.clone());
            }
        }
        if min_dis > distance_meter { None } else { ret }
    }
}

fn contains_name(fullname: &str, name: &str) -> bool {
    fullname.match_indices(name).any(|(start, _)| {
        let before = fullname[..start].chars().next_back();
        let after = fullname[start + name.len()..].chars().next();
        let starts_number = name.chars().next().is_some_and(|c| c.is_ascii_digit());
        let ends_number = name.chars().next_back().is_some_and(|c| c.is_ascii_digit());
        (!starts_number || !before.is_some_and(|c| c.is_ascii_digit()))
            && (!ends_number || !after.is_some_and(|c| c.is_ascii_digit()))
    })
}

impl IntoIterator for LocationService {
    type Item = Arc<Location>;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.locations.into_iter()
    }
}

impl<'a> IntoIterator for &'a LocationService {
    type Item = &'a Arc<Location>;
    type IntoIter = std::slice::Iter<'a, Arc<Location>>;

    fn into_iter(self) -> Self::IntoIter {
        self.locations.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(rows: serde_json::Value) -> String {
        serde_json::json!({"schema_version":1,"coordinate_system":"xmu-map-native","locations":rows}).to_string()
    }

    #[test]
    fn real_catalog_resolves_courses_and_legacy_cache_names() {
        let service = LocationService::from_json(EMBEDDED_CATALOG).unwrap();
        assert!(service.locations.len() > 400);
        for (name, region) in [
            ("翔安校区 学武楼101", Region::XiangAn),
            ("航院大楼", Region::XiangAn),
            ("思明校区 嘉庚5-101", Region::SiMing),
            ("漳州校区 图书馆", Region::ZhangZhou),
        ] {
            assert!(
                service.query(name).is_some_and(|p| p.region == region),
                "{name}"
            );
        }
        let original: serde_json::Value = serde_json::from_str(EMBEDDED_CATALOG).unwrap();
        for row in original["legacy_locations"].as_array().unwrap() {
            let name = row["name"].as_str().unwrap();
            assert!(
                service.query(name).is_some(),
                "old stored location must still resolve: {name}"
            );
        }
    }

    #[test]
    fn same_name_needs_campus_and_never_selects_an_arbitrary_site() {
        let service=LocationService::from_json(&fixture(serde_json::json!([
            {"region":"SiMing","name":"思明校区·图书馆","aliases":["图书馆"],"longitude":118.10,"latitude":24.44},
            {"region":"ZhangZhou","name":"漳州校区·图书馆","aliases":["图书馆"],"longitude":118.05,"latitude":24.37}
        ]))).unwrap();
        assert!(service.query("图书馆").is_none());
        assert!(
            service
                .query("漳州校区 图书馆")
                .is_some_and(|p| p.region == Region::ZhangZhou)
        );
        let location = service.query("思明校区 图书馆").unwrap();
        assert_eq!(service.query(location.name), Some(location));
    }

    #[test]
    fn numbered_buildings_require_a_complete_number() {
        let service=LocationService::from_json(&fixture(serde_json::json!([
            {"region":"XiangAn","name":"翔安校区·芙蓉1","aliases":["芙蓉1"],"longitude":118.31,"latitude":24.60},
            {"region":"XiangAn","name":"翔安校区·芙蓉10","aliases":["芙蓉10"],"longitude":118.32,"latitude":24.60}
        ]))).unwrap();
        assert!(
            service
                .query("芙蓉10")
                .is_some_and(|p| p.name.ends_with("10"))
        );
        assert!(service.query("芙蓉11").is_none());
    }

    #[test]
    fn swapped_coordinates_and_unknown_formats_are_rejected() {
        let invalid = fixture(
            serde_json::json!([{"region":"SiMing","name":"图书馆","longitude":24.44,"latitude":118.10}]),
        );
        assert!(LocationService::from_json(&invalid).is_err());
        let valid = fixture(
            serde_json::json!([{"region":"SiMing","name":"图书馆","longitude":118.10,"latitude":24.44}]),
        );
        assert!(
            LocationService::from_json(&valid.replace("xmu-map-native", "unknown-map")).is_err()
        );
    }

    #[test]
    fn validates_empty_duplicate_and_oversized_catalogs() {
        assert!(LocationService::from_json(&fixture(serde_json::json!([]))).is_err());
        let row = serde_json::json!({"region":"SiMing","name":"图书馆","longitude":118.10,"latitude":24.44});
        assert!(LocationService::from_json(&fixture(serde_json::json!([row, row]))).is_err());
        assert!(LocationService::from_json(&" ".repeat(1024 * 1024 + 1)).is_err());
    }

    #[test]
    fn fixed_list_constructor_and_nearest_search_remain_available() {
        let service =
            LocationService::new([Location::new(Region::XiangAn, "测试楼", 118.31, 24.60)]);
        assert_eq!(service.query("测试楼101").unwrap().name, "测试楼");
        assert!(service.get(1).is_none());
        assert!(service.find(24.60, 118.31, 100.0).is_some());
        assert!(service.find(24.50, 118.20, 100.0).is_none());
    }

    #[test]
    fn reference_triangle_does_not_depend_on_catalog_order() {
        let service = LocationService::from_json(EMBEDDED_CATALOG).unwrap();
        for name in ["学武", "西部片区2号", "一期田径场"] {
            let location = service
                .query(name)
                .expect("original reference point must remain addressable");
            assert_eq!(location.region, Region::XiangAn);
        }
    }
}
