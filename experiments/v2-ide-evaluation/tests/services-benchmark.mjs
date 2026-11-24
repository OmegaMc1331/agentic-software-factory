import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const state = path.join(root, '.factory/v2/p01');
const config = JSON.parse(fs.readFileSync(path.join(state, 'config.json'), 'utf8'));
const base = process.env.P01_BASE_URL ?? 'http://127.0.0.1:4387';
const rows = [];
for (const candidate of ['monaco', 'codemirror']) {
  for (let repetition = 1; repetition <= 3; repetition++) {
    for (const prefix of ['workspace', 'extension']) {
      const filename = path.join(config.projects.small, `.factory/p01/${prefix}-${candidate}.json`);
      if (fs.existsSync(filename)) fs.unlinkSync(filename);
    }
    fs.writeFileSync(path.join(config.projects.small, 'src/main.ts'), 'export function greet(name: string): string { return `Hello ${name}`; }\nconst wrong: number = "mistake";\n');
    const browser = await chromium.launch({ channel: 'msedge', headless: true });
    try {
      const page = await browser.newPage();
      await page.goto(`${base}/${candidate}/?project=small`);
      await page.waitForFunction(() => window.p01?.workbench.ready);
      await page.evaluate(() => window.p01.workbench.open('src/main.ts'));
      const start = performance.now();
      const lsp = await page.evaluate(() => window.p01.workbench.startLsp());
      await page.waitForFunction(() => window.p01.workbench.handles.get('src/main.ts').diagnostics().length > 0);
      const firstDiagnosticMs = performance.now() - start;
      const memory = process.platform === 'win32' ? JSON.parse(execFileSync('powershell.exe', ['-NoProfile', '-Command', `$ownedIds = @(${lsp.pid}); $all = Get-CimInstance Win32_Process; for ($i = 0; $i -lt 4; $i++) { $ownedIds += @($all | Where-Object { $ownedIds -contains $_.ParentProcessId } | Select-Object -ExpandProperty ProcessId) }; Get-Process -Id ($ownedIds | Select-Object -Unique) -ErrorAction SilentlyContinue | Select-Object Id,ProcessName,WorkingSet64,PrivateMemorySize64 | ConvertTo-Json -Compress`], { encoding: 'utf8' })) : null;
      await page.evaluate(() => window.p01.workbench.stopLsp());
      const dap = await page.evaluate(() => window.p01.workbench.startDebug());
      await page.evaluate(() => window.p01.workbench.continueDebug());
      let extension;
      if (candidate === 'codemirror') {
        await page.evaluate(() => window.p01.workbench.open('sample.yaml'));
        await page.evaluate(() => window.p01.workbench.plugin('install'));
        extension = await page.evaluate(() => window.p01.workbench.plugin('activate'));
        await page.evaluate(() => window.p01.workbench.plugin('uninstall'));
      } else extension = { status: 'unavailable: tested monaco-yaml worker incompatible with Monaco 0.57; no successful activation latency' };
      rows.push({ candidate, repetition, lsp, firstDiagnosticMs, lspProcessTreeMemory: memory, dap: { adapterReadyMs: dap.adapterReadyMs, breakpointMs: dap.breakpointMs }, extension });
      fs.writeFileSync(path.join(state, 'service-benchmarks.json'), JSON.stringify({ schemaVersion: 1, measuredAt: new Date().toISOString(), rows }, null, 2));
      console.log(`${candidate} ${repetition}: LSP ${lsp.readyMs.toFixed(1)}ms diagnostic ${firstDiagnosticMs.toFixed(1)}ms DAP ${dap.adapterReadyMs.toFixed(1)}ms`);
    } finally { await browser.close(); }
  }
}
