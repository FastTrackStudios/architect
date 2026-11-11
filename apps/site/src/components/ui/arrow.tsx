/** The action arrows: down for a download, right for a page. */
export function Arrow({ dir }: { dir: "down" | "right" }) {
	return (
		<svg className="act-arrow" viewBox="0 0 24 24" aria-hidden="true">
			<path
				d={
					dir === "down"
						? "M12 4v13M6 11l6 6 6-6M5 20h14"
						: "M5 12h13M12 6l6 6-6 6"
				}
			/>
		</svg>
	);
}
