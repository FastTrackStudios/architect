/**
 * The mark: one tapered column between its capital and its base, split
 * into an A. Two cuts of the same drawing: the full one (64 units), and a
 * small one (16 units) with heavier bars and wider gaps, for the header
 * and the favicon, where the full cut's gaps would close up.
 * public/logo.svg and public/favicon.svg are these same two paths.
 */
export const MARK = {
	full: "M17 5h30v5H17ZM22 13h20l10 41H12ZM32 30l6 24H26ZM7 57h50v4H7Z",
	small: "M4 1h8v2H4ZM5.5 4h5L13 12H3ZM8 7.5l1.600 4.500H6.400ZM1 13h14v2H1Z",
};

export function BrandMark({ size = "small" }: { size?: "small" | "full" }) {
	return (
		<svg
			className="brand-mark"
			viewBox={size === "small" ? "0 0 16 16" : "0 0 64 64"}
			aria-hidden="true"
		>
			<path fillRule="evenodd" d={MARK[size]} />
		</svg>
	);
}
