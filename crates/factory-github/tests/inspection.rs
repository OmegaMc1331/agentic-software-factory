use factory_github::{GhCli, GitHubError};
use serde_json::{json, Value};
use std::fs;

struct Provider {
    temp: tempfile::TempDir,
    gh: GhCli,
}
impl Provider {
    fn new(mode: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("provider.py");
        let state = temp.path().join("state.json");
        fs::write(
            &state,
            json!({"mode":mode,"title":"Actual fixture PR","body":"Recorded body","draft":true})
                .to_string(),
        )
        .unwrap();
        fs::write(&script,r#"import json,sys
from pathlib import Path
p=Path(sys.argv[1]);s=json.loads(p.read_text());args=sys.argv[2:]
with (p.parent/'calls.jsonl').open('a') as f:f.write(json.dumps(args)+'\n')
mode=s['mode']
if args[:2]==['auth','status']:
 if mode=='auth':print('authentication required',file=sys.stderr);sys.exit(1)
 print('Logged in to github.com account fixture (keyring)');sys.exit(0)
if mode=='rate':print('HTTP 403: API rate limit exceeded',file=sys.stderr);sys.exit(1)
if mode=='denied':print('HTTP 403: permission denied',file=sys.stderr);sys.exit(1)
if args[:2]==['pr','edit']:
 s['title']=args[args.index('--title')+1];s['body']=Path(args[args.index('--body-file')+1]).read_text();p.write_text(json.dumps(s));print('updated');sys.exit(0)
if args[:2]==['pr','ready']:
 s['draft']='--undo' in args;p.write_text(json.dumps(s));sys.exit(0)
endpoint=args[-1]
repo='foreign/repo' if mode=='mismatch' else 'owner/project'
pr={'number':12,'title':s['title'],'body':s['body'],'html_url':'https://github.com/owner/project/pull/12','state':'open','draft':s['draft'],'merged':False,'base':{'ref':'main','repo':{'full_name':repo,'id':7}},'head':{'ref':'factory/run-1','sha':'a'*40,'repo':{'full_name':'owner/project'}},'updated_at':'2026-11-08T10:00:00Z','mergeable':None,'requested_reviewers':[]}
if endpoint=='repos/owner/project':value={'id':7,'full_name':repo,'html_url':'https://github.com/owner/project','default_branch':'main','private':True,'permissions':{'push':True}}
elif '/pulls?' in endpoint:value=[pr]
elif endpoint.endswith('/pulls/12'):value=pr
elif '/files?' in endpoint:value=[{'filename':'src/service.rs','status':'modified','additions':2,'deletions':1}]
elif '/reviews?' in endpoint:value=[{'id':1,'state':'CHANGES_REQUESTED','body':'Actual provider fixture finding','user':{'login':'reviewer'}}]
elif '/comments?' in endpoint:value=[]
elif '/check-runs?' in endpoint:
 if mode=='checks-unavailable':print('HTTP 403: permission denied',file=sys.stderr);sys.exit(1)
 value={'total_count':3,'check_runs':[{'id':1,'name':'running','status':'in_progress','conclusion':None},{'id':2,'name':'cancelled','status':'completed','conclusion':'cancelled'},{'id':3,'name':'tests','status':'completed','conclusion':'success'}]}
elif '/statuses?' in endpoint:value=[{'id':1,'context':'external','state':'pending'}]
else:print('unexpected endpoint',file=sys.stderr);sys.exit(1)
print(json.dumps(value))
"#).unwrap();
        let gh = GhCli::wrap(
            if cfg!(windows) { "python" } else { "python3" },
            vec![script.into_os_string(), state.into_os_string()],
        );
        Self { temp, gh }
    }
    fn calls(&self) -> Vec<Value> {
        fs::read_to_string(self.temp.path().join("calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

#[test]
fn production_adapter_verifies_repository_identity_and_pr_pagination() {
    let p = Provider::new("normal");
    assert!(p.gh.auth_status().is_ok());
    let repo = p.gh.repository_metadata("owner/project").unwrap();
    assert_eq!(repo["id"], 7);
    assert_eq!(repo["fullName"], "owner/project");
    let page = p.gh.pull_requests_page("owner/project", 2).unwrap();
    assert_eq!(page["page"], 2);
    assert_eq!(page["items"][0]["number"], 12);
    assert_eq!(page["mayHaveMore"], false);
    assert!(p
        .calls()
        .iter()
        .any(|call| call.as_array().unwrap().iter().any(|arg| arg
            .as_str()
            .is_some_and(|text| text.contains("per_page=30&page=2")))));
}

#[test]
fn remote_unknown_running_cancelled_success_and_review_decisions_remain_distinct() {
    let p = Provider::new("normal");
    let detail = p.gh.pull_request_details("owner/project", 12, 1).unwrap();
    assert!(detail["pullRequest"]["mergeable"].is_null());
    assert_eq!(detail["checks"]["check_runs"][0]["status"], "in_progress");
    assert!(detail["checks"]["check_runs"][0]["conclusion"].is_null());
    assert_eq!(detail["checks"]["check_runs"][1]["conclusion"], "cancelled");
    assert_eq!(detail["checks"]["check_runs"][2]["conclusion"], "success");
    assert_eq!(detail["statuses"][0]["state"], "pending");
    assert_eq!(detail["reviews"][0]["state"], "CHANGES_REQUESTED");
    assert_eq!(detail["files"][0]["filename"], "src/service.rs");
    assert_eq!(detail["complete"], false);
}

#[test]
fn partial_permission_failure_is_unavailable_and_not_fabricated_success() {
    let p = Provider::new("checks-unavailable");
    let detail = p.gh.pull_request_details("owner/project", 12, 1).unwrap();
    assert!(detail["checks"].is_null());
    assert!(detail["checksError"]
        .as_str()
        .unwrap()
        .contains("permission denied"));
    assert_eq!(detail["statuses"][0]["state"], "pending");
}

#[test]
fn authentication_rate_limits_and_authorization_failures_are_classified_without_retries() {
    let auth = Provider::new("auth");
    assert!(matches!(
        auth.gh.auth_status(),
        Err(GitHubError::GhAuthRequired)
    ));
    assert_eq!(auth.calls().len(), 1);
    let rate = Provider::new("rate");
    assert!(matches!(
        rate.gh.repository_metadata("owner/project"),
        Err(GitHubError::RateLimited)
    ));
    assert_eq!(rate.calls().len(), 1);
    let denied = Provider::new("denied");
    assert!(matches!(
        denied.gh.repository_metadata("owner/project"),
        Err(GitHubError::PermissionDenied(_))
    ));
    assert_eq!(denied.calls().len(), 1);
}

#[test]
fn forged_repository_and_pr_base_identity_are_rejected() {
    let p = Provider::new("mismatch");
    assert!(matches!(
        p.gh.repository_metadata("owner/project"),
        Err(GitHubError::RemoteParse(_))
    ));
    assert!(matches!(
        p.gh.pull_request_details("owner/project", 12, 1),
        Err(GitHubError::RemoteParse(_))
    ));
}

#[test]
fn model_or_browser_strings_cannot_become_arbitrary_provider_arguments() {
    let p = Provider::new("normal");
    for repository in [
        "--help",
        "owner/project/../../secret",
        "owner/project?x=1",
        "owner/project\nother",
    ] {
        assert!(p.gh.repository_metadata(repository).is_err());
    }
    assert!(p.gh.pull_request_details("owner/project", -1, 1).is_err());
    assert!(p.gh.pull_requests_page("owner/project", 1001).is_err());
    assert!(p.calls().is_empty());
}

#[test]
fn authorized_adapter_update_preserves_multiline_body_and_verifies_remote_draft_state() {
    let p = Provider::new("normal");
    let body = "A real multiline fixture body.\n\nLiteral $() and backticks remain data.\n";
    let result =
        p.gh.update_pull_request("owner/project", 12, "Updated fixture title", body, false)
            .unwrap();
    assert_eq!(result["body"], body);
    assert_eq!(result["pullRequest"]["title"], "Updated fixture title");
    assert_eq!(result["pullRequest"]["draft"], false);
    let calls = p.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call[0] == "pr" && call[1] == "edit")
            .count(),
        1
    );
    let edit = calls
        .iter()
        .find(|call| call[0] == "pr" && call[1] == "edit")
        .unwrap();
    let args = edit.as_array().unwrap();
    let file = args[args.iter().position(|v| v == "--body-file").unwrap() + 1]
        .as_str()
        .unwrap();
    assert!(!std::path::Path::new(file).exists());
    assert!(!calls
        .iter()
        .any(|call| call[0] == "pr" && call[1] == "create"));
}
