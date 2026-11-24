const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const state = path.resolve(__dirname, '../../../.factory/v2/p01');
const result = { candidate: 'Theia', version: '1.76.0', kind: 'isolated service integration spike', fullWorkbenchTested: false, node: process.version };

async function run() {
  const fixture = path.join(state, 'theia-outside-asf-project.txt');
  fs.writeFileSync(fixture, 'P01_OWNED_OUTSIDE_PROJECT_FIXTURE');
  const { DiskFileSystemProvider } = require('@theia/filesystem/lib/node/disk-file-system-provider');
  const URI = require('@theia/core/lib/common/uri').default;
  const provider = new DiskFileSystemProvider();
  const bytes = await provider.readFile(new URI(pathToFileURL(fixture).href));
  result.providerRead = Buffer.from(bytes).toString();
  result.directNodeFilesystemAuthority = result.providerRead === 'P01_OWNED_OUTSIDE_PROJECT_FIXTURE';
  result.rustPolicyInvoked = false;
  provider.dispose();
}
run().catch(error => { result.error = String(error.stack); }).finally(() => {
  fs.writeFileSync(path.join(state, 'theia-service-spike.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
});
