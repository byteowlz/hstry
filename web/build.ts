// Build the single-file UI: tokens + styles + bundled script inlined into one
// HTML document, so it runs under a strict CSP (hstry-api) and as a
// self-contained Oqto App bundle (no network, no separate assets).
//
// Writes:
//   ../crates/hstry-api/ui/index.html        (served at GET /ui)
//   ../oqto/hstry.oqtoapp/bundle/index.html  (Oqto App presentation)

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const here = import.meta.dir;
const outputs = [
	join(here, "../crates/hstry-api/ui/index.html"),
	join(here, "../oqto/hstry.oqtoapp/bundle/index.html"),
];

const result = await Bun.build({
	entrypoints: [join(here, "src/main.ts")],
	target: "browser",
	format: "esm",
	minify: true,
});
if (!result.success) {
	for (const log of result.logs) console.error(log);
	process.exit(1);
}
const script = (await result.outputs[0]!.text()).replaceAll("</script", "<\\/script");
const tokens = readFileSync(join(here, "vendor/oqto.css"), "utf8");
const styles = readFileSync(join(here, "src/styles.css"), "utf8");
const html = readFileSync(join(here, "index.html"), "utf8")
	.replace("/*__TOKENS__*/", () => tokens)
	.replace("/*__STYLES__*/", () => styles)
	.replace("//__SCRIPT__", () => script);

for (const out of outputs) {
	mkdirSync(dirname(out), { recursive: true });
	writeFileSync(out, html);
	console.log(`wrote ${out} (${(html.length / 1024).toFixed(1)} KiB)`);
}
