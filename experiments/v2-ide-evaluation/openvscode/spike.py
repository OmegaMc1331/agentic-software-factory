import hashlib
import json
import pathlib
import platform
import subprocess
import tarfile
import urllib.request


ROOT = pathlib.Path(__file__).resolve().parents[3]
STATE = ROOT / ".factory/v2/p01"


def main():
    request = urllib.request.Request("https://api.github.com/repos/gitpod-io/openvscode-server/releases/latest", headers={"User-Agent": "ASF-P01-evaluation"})
    with urllib.request.urlopen(request, timeout=30) as response:
        release = json.load(response)
    asset = next(asset for asset in release["assets"] if asset["name"].endswith("linux-x64.tar.gz"))
    downloads = STATE / "downloads"
    downloads.mkdir(parents=True, exist_ok=True)
    archive = downloads / asset["name"]
    if not archive.exists():
        urllib.request.urlretrieve(asset["browser_download_url"], archive)
    result = {"candidate": "OpenVSCode Server", "tag": release["tag_name"], "published_at": release["published_at"], "assets": [{"name": a["name"], "bytes": a["size"]} for a in release["assets"]], "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(), "fullWorkbenchTested": False, "host": platform.platform()}
    with tarfile.open(archive) as tar:
        node = next(member for member in tar.getmembers() if member.name.endswith("/node") and member.isfile())
        contents = tar.extractfile(node).read()
        binary = downloads / "openvscode-node-linux-x64"
        binary.write_bytes(contents)
        result["node_magic_hex"] = contents[:4].hex()
        for filename in ["product.json", "package.json"]:
            member = next((m for m in tar.getmembers() if m.name.count('/') == 1 and m.name.endswith('/' + filename)), None)
            if member:
                result[filename] = json.load(tar.extractfile(member))
        if platform.system() == "Windows":
            try:
                process = subprocess.run([str(binary), "--version"], capture_output=True, timeout=5)
                result["native_launch_exit"] = process.returncode
            except OSError as error:
                result["native_launch_error"] = {"winerror": error.winerror, "message": str(error)}
    (STATE / "openvscode-spike.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps({key: value for key, value in result.items() if key not in ["product.json", "package.json"]}, indent=2))


if __name__ == "__main__":
    main()
