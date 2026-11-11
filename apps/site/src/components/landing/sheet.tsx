import type { ReactNode } from "react";

const ZONES_TOP = "12345678".split("");
const ZONES_LEFT = "ABCDEF".split("");

/**
 * One sheet of the drawing set: a section with its zone labels (1-8
 * along the top, A-F down the left) and, unless it has none, the strip at
 * its foot. landing.js added the zones after load; they are rendered here,
 * so they are in the first paint.
 */
export function Sheet({
	id,
	sheet,
	labelledBy,
	className = "sheet",
	strip,
	children,
}: {
	id: string;
	sheet: string;
	labelledBy: string;
	className?: string;
	strip?: { sn: string; t: string; s: string };
	children: ReactNode;
}) {
	return (
		<section
			className={className}
			id={id}
			aria-labelledby={labelledBy}
			data-sheet={sheet}
		>
			{children}
			{strip ? (
				<footer className="strip" aria-hidden="true">
					<span className="strip-sn">{strip.sn}</span>
					<span className="strip-t">{strip.t}</span>
					<span className="strip-s">{strip.s}</span>
				</footer>
			) : null}
			<div className="zones zones-top" aria-hidden="true">
				{ZONES_TOP.map((z) => (
					<span key={z} data-z={z} />
				))}
			</div>
			<div className="zones zones-left" aria-hidden="true">
				{ZONES_LEFT.map((z) => (
					<span key={z} data-z={z} />
				))}
			</div>
		</section>
	);
}

export function SheetHead({
	id,
	title,
	lede,
}: {
	id: string;
	title: string;
	lede: string;
}) {
	return (
		<div className="sheet-head">
			<h2 id={id}>{title}</h2>
			<p className="lede">{lede}</p>
		</div>
	);
}
