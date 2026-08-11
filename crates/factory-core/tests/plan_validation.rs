use factory_core::planner::{parse_plan, validate_plan};
use factory_models::Plan;

fn plan_json(objective: &str) -> String {
    format!(
        r#"{{"objective":"{objective}","tasks":[{{"id":"T1","title":"One","objective":"do one","dependencies":[],"acceptanceCriteria":["one works"]}},{{"id":"T2","title":"Two","objective":"do two","dependencies":["T1"],"acceptanceCriteria":["two works"]}}]}}"#
    )
}

#[test]
fn accepts_a_valid_plan() {
    let content = plan_json("edge case");
    let plan = parse_plan(&content).unwrap();
    assert_eq!(plan.tasks.len(), 2);
    assert_eq!(plan.tasks[1].dependencies, vec!["T1".to_string()]);
}

#[test]
fn accepts_a_valid_plan_json_object() {
    let plan = parse_plan(
        r#"{
            "objective": "o",
            "tasks": [
                {"id": "T1", "title": "t", "objective": "o1", "dependencies": [], "acceptanceCriteria": ["c"]}
            ]
        }"#,
    )
    .unwrap();
    assert_eq!(plan.tasks.len(), 1);
}

#[test]
fn rejects_non_json_output() {
    assert!(parse_plan("sure, I can do that").is_err());
}

#[test]
fn rejects_missing_fields() {
    assert!(parse_plan(r#"{"objective":"o"}"#).is_err());
}

#[test]
fn rejects_tasks_with_missing_ids() {
    let plan = serde_json::from_str::<Plan>(r#"{"objective":"o","tasks":[{"id":"","title":"t","objective":"o","dependencies":[],"acceptanceCriteria":["c"]}]}"#).unwrap();
    assert!(validate_plan(&plan).is_err());
}

#[test]
fn rejects_empty_titles_and_objectives() {
    let plan = serde_json::from_str::<Plan>(r#"{"objective":"o","tasks":[{"id":"T1","title":"  ","objective":"o","dependencies":[],"acceptanceCriteria":["c"]}]}"#).unwrap();
    assert!(validate_plan(&plan).is_err());
    let plan = serde_json::from_str::<Plan>(r#"{"objective":"o","tasks":[{"id":"T1","title":"t","objective":"","dependencies":[],"acceptanceCriteria":["c"]}]}"#).unwrap();
    assert!(validate_plan(&plan).is_err());
}

#[test]
fn rejects_tasks_without_acceptance_criteria() {
    let plan = serde_json::from_str::<Plan>(
        r#"{"objective":"o","tasks":[{"id":"T1","title":"t","objective":"o","dependencies":[]}]}"#,
    )
    .unwrap();
    assert!(validate_plan(&plan).is_err());
}

#[test]
fn rejects_duplicate_task_ids() {
    let plan = serde_json::from_str::<Plan>(r#"{"objective":"o","tasks":[{"id":"T1","title":"t","objective":"o","dependencies":[],"acceptanceCriteria":["c"]},{"id":"T1","title":"u","objective":"u","dependencies":[],"acceptanceCriteria":["c"]}]}"#).unwrap();
    assert!(validate_plan(&plan).is_err());
}

#[test]
fn rejects_unknown_dependencies() {
    let plan = serde_json::from_str::<Plan>(r#"{"objective":"o","tasks":[{"id":"T1","title":"t","objective":"o","dependencies":["T9"],"acceptanceCriteria":["c"]}]}"#).unwrap();
    assert!(validate_plan(&plan).is_err());
}

#[test]
fn rejects_self_dependencies() {
    let plan = serde_json::from_str::<Plan>(r#"{"objective":"o","tasks":[{"id":"T1","title":"t","objective":"o","dependencies":["T1"],"acceptanceCriteria":["c"]}]}"#).unwrap();
    assert!(validate_plan(&plan).is_err());
}

#[test]
fn rejects_cyclic_dependency_graphs() {
    let plan = serde_json::from_str::<Plan>(
        r#"{"objective":"o","tasks":[
            {"id":"T1","title":"t","objective":"o","dependencies":["T2"],"acceptanceCriteria":["c"]},
            {"id":"T2","title":"t","objective":"o","dependencies":["T1"],"acceptanceCriteria":["c"]}
        ]}"#,
    )
    .unwrap();
    assert!(validate_plan(&plan).is_err());
}

#[test]
fn normalizes_and_dedups_dependencies() {
    let plan = serde_json::from_str::<Plan>(r#"{"objective":"o","tasks":[{"id":"T1","title":"t","objective":"o","dependencies":[],"acceptanceCriteria":["c"]},{"id":"T2","title":"t","objective":"o","dependencies":["T1","T1"],"acceptanceCriteria":["c"]}]}"#).unwrap();
    let plan = factory_core::planner::normalize_plan(plan);
    assert_eq!(plan.tasks[1].dependencies, vec!["T1".to_string()]);
}

#[test]
fn accepts_backticks_fences_around_the_json() {
    // models sometimes wrap the JSON in ```json fences
    let content = "```json\n{\"objective\":\"o\",\"tasks\":[{\"id\":\"T1\",\"title\":\"t\",\"objective\":\"o\",\"dependencies\":[],\"acceptanceCriteria\":[\"c\"]}]}\n```";
    let plan = parse_plan(content).unwrap();
    assert_eq!(plan.tasks.len(), 1);
}
