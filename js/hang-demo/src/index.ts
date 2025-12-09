import "./highlight";

import { Effect } from "@kixelated/signals";
import HangSupport from "@kixelated/hang/support/element";
import HangWatch from "@kixelated/hang/watch/element";
import { Time } from "@kixelated/hang";

export { HangSupport, HangWatch };

const watch = document.querySelector("#watch") as HangWatch | null;
if (!watch) throw new Error('unable to find <hang-watch id="watch"> element');

// If query params are provided, use it as the broadcast path.
const urlParams = new URLSearchParams(window.location.search);
const path = urlParams.get("path");
if (path) {
	watch.setAttribute("path", path);
}

// --- Simple ABR selector wiring ---
const qualitySelect = document.getElementById("quality-select") as HTMLSelectElement | null;
if (qualitySelect) {
	// Reactively rebuild the quality list whenever the video catalog changes.
	const abrEffect = new Effect();
	abrEffect.effect((e) => {
		const rootCatalog = e.get(watch.broadcast.catalog);
		const videoCatalog = rootCatalog?.video;
		const renditions = videoCatalog?.renditions ?? {};

		// Clear existing options except the Auto entry.
		// Iterate backwards so removing options doesn't affect the yet-to-be-visited indices.
		for (let i = qualitySelect.options.length - 1; i >= 0; i--) {
			const option = qualitySelect.options[i];
			if (!option) continue;
			if (option.value !== "") {
				qualitySelect.remove(i);
			}
		}

		for (const [name, config] of Object.entries(renditions)) {
			const option = document.createElement("option");
			option.value = name;
			const size =
				config.codedWidth && config.codedHeight
					? ` (${config.codedWidth}x${config.codedHeight})`
					: "";
			option.textContent = `${name}${size}`;
			qualitySelect.appendChild(option);
		}
	});

	qualitySelect.addEventListener("change", () => {
		const selected = qualitySelect.value || undefined;
		const source = watch.video.source;
		source.setActiveRendition?.(selected);
	});

	// Clean up when the window unloads to avoid GC warnings.
	window.addEventListener(
		"beforeunload",
		() => {
			abrEffect.close();
		},
		{ once: true },
	);
}
