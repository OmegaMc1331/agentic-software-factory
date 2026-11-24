use factory_db::FactoryDb;
use factory_memory::MemoryService;
use factory_policy::Secrets;
use factory_types::memory::*;
use serde_json::json;
use std::{path::Path, time::Instant};
const PROJECT: &str = "p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
fn main() {
    let directory = std::env::args()
        .nth(1)
        .expect("disposable output directory required");
    let root = Path::new(&directory);
    std::fs::create_dir_all(root).unwrap();
    let database = root.join("memory.sqlite3");
    assert!(!database.exists(), "preserve earlier measurement databases");
    let db = FactoryDb::open(&database).unwrap();
    let s = MemoryService::new(&db, PROJECT, root, Secrets::new()).unwrap();
    let started = Instant::now();
    let mut samples = Vec::new();
    let mut prior = 0;
    for count in [100, 1000, 10000] {
        let create_start = Instant::now();
        let mut creates = Vec::new();
        let mut updates = Vec::new();
        for i in prior..count {
            let d = MemoryInput { category: MemoryCategory::CodingConventions, title:format!("Repository adapter contract {i}"), content:format!("Component component_{i} uses an authorized repository adapter. Its migration checks preserve backwards compatible inputs, validate boundary cases, and verify repeat requests without duplicating effects. Recorded fixture fact for retrieval, not an enforceable permission."),subject:format!("component_{i}"),scope:format!("component_{i}"),status:MemoryStatus::Active,sources:vec![MemorySource{project_id:PROJECT.into(),reference:MemorySourceRef::Manual{note:"Explicit synthetic benchmark fact; no model or production data".into()}}]};
            let t = Instant::now();
            let mut m = s.create(&d, "performance fixture").unwrap();
            creates.push(t.elapsed().as_secs_f64() * 1000.0);
            for revision in 2..=3 {
                let mut next = d.clone();
                next.content
                    .push_str(&format!(" Regression finding revision {revision}."));
                let t = Instant::now();
                m = s
                    .update(&m.id, m.revision, &next, "fixture revision")
                    .unwrap();
                updates.push(t.elapsed().as_secs_f64() * 1000.0);
            }
        }
        let load_ms = create_start.elapsed().as_secs_f64() * 1000.0;
        let mut searches = Vec::new();
        let mut retrievals = Vec::new();
        let mut lists = Vec::new();
        let mut histories = Vec::new();
        for i in 0..20 {
            let query = format!("component_{} adapter", i * 7 % count);
            let t = Instant::now();
            let page = s
                .search(&MemorySearch {
                    query: query.clone(),
                    limit: Some(20),
                    ..Default::default()
                })
                .unwrap();
            searches.push(t.elapsed().as_secs_f64() * 1000.0);
            assert!(!page.items.is_empty());
            let t = Instant::now();
            let context = s.retrieve(&query, "worker", None).unwrap();
            retrievals.push(t.elapsed().as_secs_f64() * 1000.0);
            assert!(!context.records.is_empty());
            assert!(serde_json::to_vec(&context).unwrap().len() <= 12000);
            let t = Instant::now();
            let listing = s
                .search(&MemorySearch {
                    limit: Some(50),
                    ..Default::default()
                })
                .unwrap();
            lists.push(t.elapsed().as_secs_f64() * 1000.0);
            let t = Instant::now();
            let revisions = db
                .memory_revisions(PROJECT, &listing.items[0].id, None)
                .unwrap();
            histories.push(t.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(revisions.len(), 3);
        }
        let mut rebuilds = Vec::new();
        for _ in 0..3 {
            let t = Instant::now();
            db.memory_rebuild_index(PROJECT).unwrap();
            rebuilds.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let concurrency = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for _ in 0..4 {
                let database = &database;
                handles.push(scope.spawn(move || {
                    let db = FactoryDb::open(database).unwrap();
                    let service = MemoryService::new(&db, PROJECT, root, Secrets::new()).unwrap();
                    let t = Instant::now();
                    for _ in 0..10 {
                        assert!(!service
                            .retrieve("repository adapter", "planner", None)
                            .unwrap()
                            .records
                            .is_empty());
                    }
                    t.elapsed().as_secs_f64() * 1000.0
                }));
            }
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        samples.push(json!({"count":count,"revisionCount":count*3,"loadMs":load_ms,"createMs":creates,"updateMs":updates,"searchMs":searches,"retrievalMs":retrievals,"listMs":lists,"revisionLookupMs":histories,"rebuildMs":rebuilds,"concurrentRetrievalBatchMs":concurrency,"databaseBytes":std::fs::metadata(&database).unwrap().len(),"walBytes":std::fs::metadata(root.join("memory.sqlite3-wal")).map(|m|m.len()).unwrap_or(0)}));
        prior = count;
    }
    let value = json!({"schema":1,"kind":"synthetic-local-service-measurement-not-model-throughput","project":PROJECT,"elapsedMs":started.elapsed().as_secs_f64()*1000.0,"samples":samples});
    std::fs::write(
        root.join("performance.json"),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();
    println!("performance recorded in {}", root.display());
}
