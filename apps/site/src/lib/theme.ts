import { SITE } from "#/content/site";

/**
 * Plot (light) and Model (dark). The choice is kept under SITE.themeKey and
 * set on <html data-theme> before first paint by THEME_SCRIPT (in the
 * root's head), so a reload never flashes the other drawing. With no
 * choice kept, the system's preference decides.
 */
export type Drawing = "plot" | "model";

/** Runs in <head> before paint: the `js` class and the kept drawing. */
export const THEME_SCRIPT = `(function(){var r=document.documentElement;r.classList.add("js");try{var t=localStorage.getItem(${JSON.stringify(SITE.themeKey)});if(t==="plot"||t==="model")r.dataset.theme=t}catch(e){}})();`;

export function currentDrawing(): Drawing {
	const t = document.documentElement.dataset.theme;
	if (t === "plot" || t === "model") return t;
	return window.matchMedia("(prefers-color-scheme: dark)").matches
		? "model"
		: "plot";
}

export function setDrawing(next: Drawing) {
	document.documentElement.dataset.theme = next;
	try {
		localStorage.setItem(SITE.themeKey, next);
	} catch {
		// Private windows and blocked storage: the choice lasts the page.
	}
	paintThemeColor();
}

/** Both theme-color metas follow the drawing in use (as landing.js did). */
export function paintThemeColor() {
	const color = SITE.themeColor[currentDrawing()];
	for (const m of document.querySelectorAll('meta[name="theme-color"]')) {
		m.setAttribute("content", color);
	}
}
