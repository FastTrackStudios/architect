import { useEffect, useState } from "react";

import { LampIcon } from "#/components/ui/icons";
import { SHELL } from "#/content/site";
import { currentTheme, paintThemeColor, setTheme } from "#/lib/theme";

/** The light/dark switch: pressed means the dark theme (the stage). */
export function ThemeToggle() {
	const [dark, setDark] = useState(false);

	useEffect(() => {
		const sync = () => {
			setDark(currentTheme() === "dark");
			paintThemeColor();
		};
		sync();
		const system = window.matchMedia("(prefers-color-scheme: dark)");
		system.addEventListener("change", sync);
		return () => system.removeEventListener("change", sync);
	}, []);

	return (
		<button
			className="theme-toggle"
			type="button"
			aria-pressed={dark}
			aria-label={dark ? SHELL.theme.toLight : SHELL.theme.toDark}
			onClick={() => {
				const next = currentTheme() === "dark" ? "light" : "dark";
				setTheme(next);
				setDark(next === "dark");
			}}
		>
			<LampIcon />
		</button>
	);
}
