// Collect license texts shipped with the application and standalone tools.
import { execFileSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
const root = resolve(import.meta.dirname, '..');
const output = join(root, 'tools/media/licenses');
mkdirSync(output, { recursive: true });
const index = [];
function collect(kind, name, version, license, folder, source) {
  const dest = join(output, kind, `${name.replace(/[^a-z\d._-]/gi, '_')}-${version}`);
  const texts = readdirSync(folder, { withFileTypes: true }).filter((f) => f.isFile() && /^(licen[sc]e|copying|notice|copyright)([._-]|$)/i.test(f.name));
  if (texts.length) mkdirSync(dest, { recursive: true });
  for (const file of texts) cpSync(join(folder, file.name), join(dest, file.name));
  index.push({ kind, name, version, license, source, texts: texts.map((t) => t.name) });
}
const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--format-version', '1', '--locked', '--filter-platform', 'x86_64-pc-windows-msvc'], { cwd: root, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024, windowsHide: true }));
for (const pkg of metadata.packages.filter((p) => p.source)) collect('rust', pkg.name, pkg.version, pkg.license, dirname(pkg.manifest_path), pkg.repository || pkg.source);
const pnpm = join(root, 'apps/desktop/node_modules/.pnpm');
for (const entry of readdirSync(pnpm, { withFileTypes: true }).filter((f) => f.isDirectory())) {
  const modules = join(pnpm, entry.name, 'node_modules');
  if (!existsSync(modules)) continue;
  const folders = readdirSync(modules, { withFileTypes: true }).filter((f) => f.isDirectory() && f.name !== '.bin').flatMap((f) => f.name.startsWith('@') ? readdirSync(join(modules, f.name)).map((n) => join(modules, f.name, n)) : [join(modules, f.name)]);
  for (const folder of folders) {
    if (!existsSync(join(folder, 'package.json'))) continue;
    const pkg = JSON.parse(readFileSync(join(folder, 'package.json'), 'utf8'));
    collect('npm', pkg.name, pkg.version, pkg.license, folder, typeof pkg.repository === 'object' ? pkg.repository.url : pkg.repository);
  }
}
writeFileSync(join(output, 'index.json'), JSON.stringify(index, null, 2) + '\n');
cpSync(join(root, 'docs/THIRD_PARTY_NOTICES.md'), join(root, 'tools/media/THIRD_PARTY_NOTICES.md'));
console.log(`Collected notices for ${index.length} dependencies.`);
