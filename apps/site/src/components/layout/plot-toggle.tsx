import { useEffect, useState } from "react";

import { BAR } from "#/content/landing";
import { currentDrawing, paintThemeColor, setDrawing } from "#/lib/theme";

/** The Plot/Model switch: pressed means the dark drawing (Model). */
export function PlotToggle() {
	const [model, setModel] = useState(false);

	useEffect(() => {
		const sync = () => {
			setModel(currentDrawing() === "model");
			paintThemeColor();
		};
		sync();
		const dark = window.matchMedia("(prefers-color-scheme: dark)");
		dark.addEventListener("change", sync);
		return () => dark.removeEventListener("change", sync);
	}, []);

	return (
		<button
			className="plot-toggle"
			type="button"
			data-plot-toggle
			aria-pressed={model}
			onClick={() => {
				const next = currentDrawing() === "model" ? "plot" : "model";
				setDrawing(next);
				setModel(next === "model");
			}}
		>
			<span className="plot-toggle-a">{BAR.plot}</span>
			<span className="plot-toggle-b">{BAR.model}</span>
			<span className="visually-hidden">{BAR.plotHint}</span>
		</button>
	);
}
