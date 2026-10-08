const fs = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '..');
const inputs = path.join(__dirname, 'location-sources');
const output = path.join(root, 'app_data/locations.json');
const read = name => JSON.parse(fs.readFileSync(path.join(inputs,name),'utf8'));
const legacy = read('legacy-locations.json');
if (!Array.isArray(legacy) || legacy.length !== 35) throw new Error('Original location evidence is missing');
const campuses = [
  {region:'XiangAn', name:'翔安校区', id:377508, file:'xmu-map-buildings.json'},
  {region:'SiMing', name:'思明校区', id:377507, file:'xmu-map-buildings-siming.json'},
  {region:'ZhangZhou', name:'漳州校区', id:377509, file:'xmu-map-buildings-zhangzhou.json'},
];
const locations = [];
const endpoint = 'https://map.xmu.edu.cn/openmap/mapi/bd/v1/list';
const numeral = {'一':'1','二':'2','三':'3','四':'4','五':'5','六':'6','七':'7','八':'8','九':'9'};
for (const campus of campuses) {
  const response = read(campus.file);
  if (response.status !== 0 || !Array.isArray(response.result) || response.result.length < 10) throw new Error('Incomplete official campus response');
  const repeated = new Map();
  for (const row of response.result) repeated.set(row.name,(repeated.get(row.name)||0)+1);
  for (const row of response.result) {
    const longitude = row.location?.x, latitude = row.location?.y;
    if (row.campusId !== campus.id || !Number.isFinite(longitude) || !Number.isFinite(latitude) || longitude < 117.9 || longitude > 118.5 || latitude < 24.25 || latitude > 24.8) throw new Error('Invalid official building coordinates');
    const title = row.name.trim();
    const base = title.split(/[（(]/)[0].trim();
    const aliases = new Set([title, base]);
    if (base.endsWith('楼') && base.length > 2) aliases.add(base.slice(0,-1));
    const brackets = title.match(/[（(]([^）)]+)[）)]/);
    if (brackets && /^(嘉庚|南强)[一二三四五六七八九]$/.test(brackets[1])) {
      const label = brackets[1], number = label.slice(0,-1)+numeral[label.slice(-1)];
      for (const alias of [label, label+'号楼', number, number+'号楼']) aliases.add(alias);
    }
    if (title === '航空航天学院') aliases.add('航院大楼');
    const suffix = repeated.get(row.name)>1 ? `（地图编号${row.id}）` : '';
    locations.push({region:campus.region, name:campus.name+'·'+title+suffix, aliases:[...aliases], longitude, latitude, source_url:endpoint, source_id:row.id, source_campus_id:campus.id});
  }
}
let mergedLegacy = 0;
for (const old of legacy) {
  const matches = locations.filter(row => row.region===old.region && row.aliases.includes(old.name));
  if (matches.length === 1) {
    const row = matches[0];
    if (!row.aliases.includes(old.name)) row.aliases.push(old.name);
    mergedLegacy++;
  } else {
    const campus = campuses.find(row=>row.region===old.region);
    locations.push({...old,name:campus.name+'·'+old.name,aliases:[old.name],source_url:'https://github.com/vintcessun/xmu_assistant_sign_bot/blob/5dc956b0fb80ed693ae5c2b4f2b6c823c74b3bee/src/api/xmu_service/location.rs',source_id:null,source_campus_id:campus.id});
  }
}
const metadata = read('metadata.json');
const data = {
  schema_version:1,
  coordinate_system:'xmu-map-native',
  source_url:'https://map.xmu.edu.cn/',
  retrieved_at:metadata.retrieved_at,
  coordinate_note:'Preserves official map coordinates and original bot coordinates; no cross-provider conversion or classroom accuracy claim.',
  legacy_locations:legacy,
  locations,
};
const json = JSON.stringify(data,null,2)+'\n';
const targets = [output];
if (process.argv.includes('--check')) {
  for (const target of targets) {
    if (JSON.stringify(JSON.parse(fs.readFileSync(target,'utf8'))) !== JSON.stringify(data)) {
      throw new Error('Catalog differs from saved public inputs: '+path.relative(root,target));
    }
  }
} else {
  for (const target of targets) fs.writeFileSync(target,json);
}
const summary = {time:data.retrieved_at,passed:true,official_locations:460,legacy_entries_preserved:legacy.length,legacy_aliases_merged:mergedLegacy,loaded_locations:locations.length,regions:Object.fromEntries(campuses.map(c=>[c.name,locations.filter(r=>r.region===c.region).length])),source_url:data.source_url};
process.stdout.write(JSON.stringify(summary,null,2));
