import hashlib,json,re,shutil,sys,platform
from pathlib import Path
from datetime import datetime,timezone
sys.stdout.reconfigure(encoding="utf8")
root=Path(__file__).resolve().parents[2]
local=root/".factory/v2/p10"
out=root/"docs/v2/evidence/p10"
out.mkdir(parents=True,exist_ok=True)
(out/".gitattributes").write_text("* -text\n",encoding="utf8")
(out/".gitignore").write_text("!logs/\n!logs/*.log\n",encoding="utf8")
def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def dump(name,value):
 (out/name).parent.mkdir(parents=True,exist_ok=True)
 (out/name).write_text(json.dumps(value,indent=2,ensure_ascii=False)+"\n",encoding="utf8")
def redact(value):
 if isinstance(value,list): return [redact(v) for v in value]
 if isinstance(value,dict): return {k:(hashlib.sha256(str(v).encode()).hexdigest() if k in {"leaseId","clientInstanceId"} else redact(v)) for k,v in value.items()}
 return value
for mode in ["native","controlled-5","controlled-final","replay"]:
 source=local/mode/"results.json"
 if source.exists():
  value=json.loads(source.read_text(encoding="utf8"))
  dump(mode+".json",redact(value))
  dest=out/mode
  dest.mkdir(exist_ok=True)
  for filename in value.get("screenshots",[]):
   src=local/mode/filename["file"];target=dest/Path(filename["file"]).name
   shutil.copyfile(src,target)
  for src in (local/mode).glob("diff-*.patch"): shutil.copyfile(src,dest/src.name)
preflight=json.loads((local/"preflight/preflight.json").read_text(encoding="utf8"))
dump("cli-preflight.json",{k:v for k,v in preflight.items() if k not in {"model_response"}})
gates=[]
for filename in sorted((local/"logs").glob("*-results.json")):
 value=json.loads(filename.read_text(encoding="utf8"))
 for record in value.get("commands",[]):
  log=local/"logs"/record["log"]
  if not log.exists():continue
  content=log.read_text(encoding="utf8",errors="replace")
  matches=re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored",content)
  item={k:record.get(k) for k in ["command","cwd","exit","durationMs","status"]}
  item.update({"log":log.name,"sha256":sha(log),"rustPassed":sum(int(m[0]) for m in matches),"rustIgnored":sum(int(m[2]) for m in matches),"memorySamples":record.get("memory",[])})
  gates.append(item)
  (out/"logs").mkdir(exist_ok=True)
  shutil.copyfile(log,out/"logs"/log.name)
dump("quality-gates.json",gates)
for name in ["frontend-assets.json","performance.json","source-equivalence.json","binary-final.json","environment.json","guard-regression-proof.json","provider-usage.json"]:
 source=local/name
 if source.exists(): shutil.copyfile(source,out/name)
source=local/"browser-regressions-qualified.json"
if source.exists():dump("browser-regressions.json",json.loads(source.read_text(encoding="utf8")))
for folder,name in [("regressions","results.json"),("regressions-final","results.json"),("p05-diagnostic","editor-browser-results.json"),("p05-synchronized","editor-browser-results.json"),("p07-recovery-label","recovery-browser-results.json")]:
 source=local/folder/name
 if source.exists():dump("browser-attempts/"+folder+"-"+name,json.loads(source.read_text(encoding="utf8")))
for folder in ["regressions/p05","p05-diagnostic"]:
 source=local/folder/"screenshots/failure.png"
 if source.exists():
  target=out/"browser-attempts"/(folder.replace("/","-")+"-failure.png")
  target.parent.mkdir(parents=True,exist_ok=True)
  shutil.copyfile(source,target)
for name in ["guard-regressions-before.utf8.log"]:
 source=local/"logs"/name
 if source.exists(): shutil.copyfile(source,out/"logs"/name)
failures=[]
for folder in sorted(local.glob("controlled*")):
 source=folder/"results.json"
 if source.exists():
  value=json.loads(source.read_text(encoding="utf8"));failures.append({"folder":folder.name,"sha256":sha(source),"status":value.get("status"),"error":value.get("error"),"modelInvocations":0,"timeline":value.get("timeline",[])})
for source in sorted((local/"logs/attempts").glob("*-*.json")):
 value=json.loads(source.read_text(encoding="utf8"))
 for c in value.get("commands",[]):
  log=local/"logs"/c["log"]
  if log.exists():
   (out/"logs").mkdir(exist_ok=True)
   shutil.copyfile(log,out/"logs"/log.name)
 failures.append({"file":str(source.relative_to(root)),"sha256":sha(source),"commands":[{k:c.get(k) for k in ["command","exit","status","log"]} for c in value.get("commands",[])]})
dump("retained-attempts.json",failures)
artifacts=[]
for path in sorted(out.rglob("*")):
 if path.is_file() and path.name!="manifest.json":artifacts.append({"path":str(path.relative_to(out)).replace("\\","/"),"sha256":sha(path),"bytes":path.stat().st_size})
dump("manifest.json",{"schema":1,"createdAt":datetime.now(timezone.utc).isoformat(),"scope":"Local Windows evidence only; no remote CI or publication","artifacts":artifacts})
print(json.dumps({"artifacts":len(artifacts),"gates":len(gates)},ensure_ascii=False))
