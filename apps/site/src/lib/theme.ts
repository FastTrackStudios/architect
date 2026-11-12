import { SITE } from "#/content/site";

/**
 * Light (the lightbox) and dark (the stage). The choice is kept under
 * SITE.themeKey and set on <html data-theme> before first paint by
 * THEME_SCRIPT (in the root's head), so a reload never flashes the other
 * theme. With no choice kept, the system's preference decides. The old
 * page kept "plot" and "model" under the same key: read as light and dark.
 */
export type Theme = "light" | "dark";

/**
 * Runs in <head> before paint: the `js` class, the kept theme, and the
 * platform (`data-os`: "linux" or "macos", the two with a desktop build),
 * which picks the Download button's target and icon in CSS.
 */
export const THEME_SCRIPT = `(function(){var r=document.documentElement;r.classList.add("js");try{var t=localStorage.getItem(${JSON.stringify(SITE.themeKey)});if(t==="plot")t="light";if(t==="model")t="dark";if(t==="light"||t==="dark")r.dataset.theme=t}catch(e){}var n=navigator,p=(n.userAgentData&&n.userAgentData.platform)||n.platform||"",o=/Android|iPhone|iPad/.test(n.userAgent)||n.maxTouchPoints>1?"":/Mac/.test(p)?"macos":/Linux|X11/.test(p)?"linux":"";if(o)r.dataset.os=o})();`;

export function currentTheme(): Theme {
	const t = document.documentElement.dataset.theme;
	if (t === "light" || t === "dark") return t;
	return window.matchMedia("(prefers-color-scheme: dark)").matches
		? "dark"
		: "light";
}

export function setTheme(next: Theme) {
	document.documentElement.dataset.theme = next;
	try {
		localStorage.setItem(SITE.themeKey, next);
	} catch {
		// Private windows and blocked storage: the choice lasts the page.
	}
	paintThemeColor();
}

/** Both theme-color metas follow the theme in use. */
export function paintThemeColor() {
	const color = SITE.themeColor[currentTheme()];
	for (const m of document.querySelectorAll('meta[name="theme-color"]')) {
		m.setAttribute("content", color);
	}
}
