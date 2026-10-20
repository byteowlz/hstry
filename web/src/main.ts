// Boot: pick the data source and the theme feed for where we are running.
//
// - Inside an Oqto host frame: connect through the App SDK, take the host's
//   live theme tokens, and read data through the pinned hstry operations.
// - Standalone (served by hstry-api): follow the OS light/dark preference over
//   the Design Studio's generated oqto tokens, and read over HTTP.

import { applyOqtoTheme, connectOqtoApp } from "@byteowlz/oqto-app-sdk";
import { HstryApp } from "./app";
import { HttpSource, OqtoSource } from "./sources";
import type { DataSource } from "./types";

function followSystemScheme(): void {
	const media = window.matchMedia("(prefers-color-scheme: dark)");
	const apply = () => document.documentElement.classList.toggle("dark", media.matches);
	apply();
	media.addEventListener("change", apply);
}

async function connectSource(): Promise<DataSource> {
	if (window.parent === window) {
		followSystemScheme();
		return new HttpSource();
	}
	try {
		const host = await connectOqtoApp();
		const root = document.documentElement;
		const theme = await host.theme?.get();
		if (theme) {
			applyOqtoTheme(root, theme);
			root.classList.toggle("dark", theme.colorScheme === "dark");
			await host.theme?.watch((next) => {
				applyOqtoTheme(root, next);
				root.classList.toggle("dark", next.colorScheme === "dark");
			});
		}
		return new OqtoSource(host);
	} catch {
		// Embedded by something that is not an Oqto host: behave standalone.
		followSystemScheme();
		return new HttpSource();
	}
}

const mount = document.getElementById("app");
if (mount) {
	const source = await connectSource();
	const app = new HstryApp(mount, source);
	await app.start();
}
