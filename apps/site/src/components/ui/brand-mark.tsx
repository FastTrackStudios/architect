/** The mark: an A drawn as a truss, nodes at its three corners. */
export function BrandMark() {
	return (
		<svg className="brand-mark" viewBox="0 0 32 32" aria-hidden="true">
			<path d="M4 28 L16 4 L28 28 M9 19 H23" fill="none" />
			<circle cx="16" cy="4" r="2.6" />
			<circle cx="4" cy="28" r="2.6" />
			<circle cx="28" cy="28" r="2.6" />
		</svg>
	);
}
