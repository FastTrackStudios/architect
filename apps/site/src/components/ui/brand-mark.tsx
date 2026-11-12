/** The mark: an A drawn as a compass, a rivet at each of its three points. */
export function BrandMark() {
	return (
		<svg className="brand-mark" viewBox="0 0 32 32" aria-hidden="true">
			<path d="M5 27 L16 5 L27 27 M10 18 H22" />
			<circle cx="16" cy="5" r="2.4" />
			<circle cx="5" cy="27" r="2.4" />
			<circle cx="27" cy="27" r="2.4" />
		</svg>
	);
}
