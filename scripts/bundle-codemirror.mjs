import { readFile, mkdir, readdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { build } from "esbuild";

const output = "ui/vendor/codemirror.js";
const result = await build({
  entryPoints: ["ui/live-editor.mjs"],
  bundle: true,
  platform: "browser",
  format: "iife",
  target: "es2022",
  minify: true,
  metafile: true,
  write: false,
  outfile: output,
});
const packages = new Set();
for (const path of Object.keys(result.metafile.inputs)) {
  const normalized = path.replaceAll("\\", "/");
  const match = /^(.*node_modules\/(?:@[^/]+\/)?[^/]+)\//.exec(normalized);
  if (match) packages.add(match[1]);
}
const notices = [];
for (const dir of [...packages].sort()) {
  const { name, version } = JSON.parse(await readFile(join(dir, "package.json"), "utf8"));
  const file = (await readdir(dir)).find((item) => /^license(?:\..*)?$/i.test(item));
  if (!file) throw new Error("License missing from " + name);
  const license = (await readFile(join(dir, file), "utf8")).replaceAll("\r\n", "\n").trim();
  notices.push(name + " " + version + "\n" + license);
}
const bundle = result.outputFiles[0].text + "\n/*! Third-party licenses\n" +
  notices.join("\n\n").replaceAll("*/", "* /") + "\n*/\n";
if (process.argv.includes("--check")) {
  if (bundle !== (await readFile(output, "utf8")).replaceAll("\r\n", "\n")) {
    throw new Error("The committed CodeMirror bundle is stale. Run npm run build:live.");
  }
} else {
  await mkdir("ui/vendor", { recursive: true });
  await writeFile(output, bundle);
}
