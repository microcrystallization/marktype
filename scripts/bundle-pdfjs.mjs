import { mkdir, readFile, writeFile } from "node:fs/promises";
import { build } from "esbuild";

const normalize = (text) => text.replaceAll("\r\n", "\n");
const license = normalize(await readFile("node_modules/pdfjs-dist/LICENSE", "utf8")).trim();
const notice = "\n/*! PDF.js license\n" + license.replaceAll("*/", "* /") + "\n*/\n";

const libOutput = "ui/vendor/pdf.js";
const libResult = await build({
  entryPoints: ["node_modules/pdfjs-dist/legacy/build/pdf.min.mjs"],
  bundle: true,
  platform: "browser",
  format: "iife",
  globalName: "pdfjsLib",
  target: "es2022",
  minify: true,
  write: false,
  outfile: libOutput,
});
const libBundle = normalize(libResult.outputFiles[0].text) + notice;

const workerOutput = "ui/vendor/pdf.worker.mjs";
const workerSource = await readFile("node_modules/pdfjs-dist/legacy/build/pdf.worker.min.mjs", "utf8");
const workerBundle = normalize(workerSource.replace(/\n?\/\/# sourceMappingURL=.*$/, "")) + notice;

if (process.argv.includes("--check")) {
  if (libBundle !== normalize(await readFile(libOutput, "utf8"))) {
    throw new Error("The committed PDF.js bundle is stale. Run npm run build:pdf.");
  }
  if (workerBundle !== normalize(await readFile(workerOutput, "utf8"))) {
    throw new Error("The committed PDF.js worker bundle is stale. Run npm run build:pdf.");
  }
} else {
  await mkdir("ui/vendor", { recursive: true });
  await writeFile(libOutput, libBundle);
  await writeFile(workerOutput, workerBundle);
}
