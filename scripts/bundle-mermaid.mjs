import { mkdir, readFile, writeFile } from "node:fs/promises";

const output = "ui/vendor/mermaid.js";
const source = await readFile("node_modules/mermaid/dist/mermaid.min.js", "utf8");
const license = await readFile("node_modules/mermaid/LICENSE", "utf8");
const normalize = (text) => text.replaceAll("\r\n", "\n");
const bundle = normalize(source.replace(/\n?\/\/# sourceMappingURL=.*$/, "")) +
  "\n/*! Mermaid license\n" + normalize(license).replaceAll("*/", "* /") + "\n*/\n";
if (process.argv.includes("--check")) {
  if (bundle !== normalize(await readFile(output, "utf8"))) throw Error("The bundled Mermaid renderer is stale");
} else {
  await mkdir("ui/vendor", { recursive: true });
  await writeFile(output, bundle);
}
