use xmu_location_tests::location::LOCATIONS;

fn main() {
    println!(
        "{}",
        serde_json::json!({
            "count": LOCATIONS.locations.len(),
            "external_file_loaded": LOCATIONS.external_file_loaded,
        })
    );
}
