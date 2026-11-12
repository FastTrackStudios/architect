import { useEffect, useRef } from "react";

import { MARK } from "#/components/ui/brand-mark";

/**
 * The figures: one line drawing per pillar, animated in CSS
 * (styles/figures.css) to show the claim rather than a screenshot of it.
 * Each is plain SVG in the page's own ink, so it follows the theme. A
 * figure animates while it is on screen; under reduced motion it is the
 * finished drawing, still.
 */
export type FigureId =
	| "modular"
	| "local-first"
	| "synced"
	| "collaborative"
	| "shared"
	| "extensions"
	| "platforms"
	| "embeddable";

const FIGURES: Record<
	FigureId,
	{ label: string; draw: () => React.ReactNode }
> = {
	modular: {
		label:
			"Four blocks on one foundation. One block lifts out and another drops into its place.",
		draw: Modular,
	},
	"local-first": {
		label:
			"A note being written. The network drops, and the writing carries on.",
		draw: Local,
	},
	synced: {
		label:
			"A node in the middle with a desktop, a laptop and a phone around it. Changes travel along the lines between them, in both directions.",
		draw: Synced,
	},
	shared: {
		label:
			"Six notes in a vault. Three are selected, and only those three appear on a published page beside it.",
		draw: Shared,
	},
	collaborative: {
		label:
			"Two people's cursors in one note, one selecting a line while the other writes.",
		draw: Collab,
	},
	extensions: {
		label:
			"Rust, TypeScript, Lua and WASM each flow into one sandbox, and come out as an extension.",
		draw: Extensions,
	},
	platforms: {
		label:
			"A desktop, a browser and a phone showing the same workspace; a new line appears on all three at once.",
		draw: Platforms,
	},
	embeddable: {
		label:
			"A window labelled your app, with an empty slot. An Architect view slides into the slot.",
		draw: Embed,
	},
};

export function Figure({ id }: { id: FigureId }) {
	const ref = useRef<HTMLDivElement>(null);
	// Run only while on screen: a looping drawing nobody sees is wasted work.
	useEffect(() => {
		const el = ref.current;
		if (!el || !("IntersectionObserver" in window)) return;
		const seen = new IntersectionObserver(
			([en]) => el.toggleAttribute("data-off", !en.isIntersecting),
			{ threshold: 0.2 },
		);
		seen.observe(el);
		return () => seen.disconnect();
	}, []);
	const f = FIGURES[id];
	return (
		<div ref={ref} className={`fig fig-${id}`}>
			<svg viewBox="0 0 480 360" role="img" aria-label={f.label}>
				{f.draw()}
			</svg>
		</div>
	);
}

/** An isometric slab: its top face at (cx, cy), half-width w, thickness t. */
function Slab({
	cx,
	cy,
	w,
	t,
	className,
}: {
	cx: number;
	cy: number;
	w: number;
	t: number;
	className?: string;
}) {
	const h = w / 2;
	return (
		<g className={className}>
			<path
				d={`M${cx - w} ${cy}L${cx} ${cy + h}V${cy + h + t}L${cx - w} ${cy + t}Z`}
			/>
			<path
				d={`M${cx + w} ${cy}L${cx} ${cy + h}V${cy + h + t}L${cx + w} ${cy + t}Z`}
			/>
			<path
				d={`M${cx} ${cy - h}L${cx + w} ${cy}L${cx} ${cy + h}L${cx - w} ${cy}Z`}
			/>
		</g>
	);
}

function Modular() {
	const w = 46;
	const t = 42;
	return (
		<>
			<Slab cx={240} cy={206} w={124} t={18} className="k" />
			<Slab cx={240} cy={160 - t + 23} w={w} t={t} />
			<Slab cx={286} cy={183 - t + 23} w={w} t={t} />
			<Slab cx={194} cy={183 - t + 23} w={w} t={t} />
			<Slab cx={240} cy={206 - t + 23} w={w} t={t} className="m-out" />
			<Slab cx={240} cy={206 - t + 23} w={w} t={t} className="k m-in" />
		</>
	);
}

function Local() {
	const lines = [110, 84, 118, 96, 60, 104];
	return (
		<>
			<rect x="150" y="70" width="170" height="220" rx="10" />
			{lines.map((len, i) => (
				<path
					// biome-ignore lint/suspicious/noArrayIndexKey: fixed lines
					key={i}
					className="k draw"
					style={{ "--d": `${i * 0.9}s` } as React.CSSProperties}
					pathLength={1}
					d={`M176 ${108 + i * 28}h${len}`}
				/>
			))}
			<g className="net">
				<path d="M352 104a34 34 0 0 1 48 0" />
				<path d="M362 116a20 20 0 0 1 28 0" />
				<circle className="dot" cx="376" cy="130" r="3" />
			</g>
			<path className="k net-off" pathLength={1} d="M350 138L402 92" />
			<text className="net-label" x="376" y="164">
				offline
			</text>
		</>
	);
}

const CURSOR = "M0 0V17L4.6 12.6 7.6 19.6 10.2 18.4 7.2 11.6H13.4Z";

function Collab() {
	return (
		<>
			<rect x="110" y="60" width="260" height="240" rx="10" />
			<path d="M140 100h150M140 128h190M140 184h120M140 240h170" />
			<rect className="sel" x="140" y="145" width="170" height="22" rx="4" />
			<path className="k" d="M140 156h170" />
			<path className="k draw" pathLength={1} d="M140 212h150" />
			<g className="cur cur-a">
				<path className="solid" d={CURSOR} />
				<g transform="translate(14 18)">
					<rect className="solid" width="40" height="20" rx="6" />
					<text className="on" x="20" y="14">
						Ana
					</text>
				</g>
			</g>
			<g className="cur cur-b">
				<path className="k hollow" d={CURSOR} />
				<g transform="translate(14 18)">
					<rect className="k hollow" width="34" height="20" rx="6" />
					<text x="17" y="14">
						Jo
					</text>
				</g>
			</g>
		</>
	);
}

function Extensions() {
	const langs = ["Rust", "TypeScript", "Lua", "WASM"];
	return (
		<>
			{langs.map((l, i) => {
				const y = 66 + i * 62;
				return (
					<g key={l}>
						<rect x="34" y={y} width="96" height="32" rx="8" />
						<text x="82" y={y + 20}>
							{l}
						</text>
						<path d={`M130 ${y + 16}C190 ${y + 16} 170 180 228 180`} />
						<circle
							className="flow"
							r="3.5"
							style={
								{
									offsetPath: `path("M130 ${y + 16}C190 ${y + 16} 170 180 228 180")`,
									"--d": `${i * 0.7}s`,
								} as React.CSSProperties
							}
						/>
					</g>
				);
			})}
			<rect className="k" x="228" y="138" width="84" height="84" rx="12" />
			<text className="strong" x="270" y="184">
				sandbox
			</text>
			<path d="M312 180h52" />
			<circle
				className="flow out"
				r="3.5"
				style={{ offsetPath: 'path("M312 180h52")' } as React.CSSProperties}
			/>
			<Slab cx={410} cy={168} w={40} t={36} className="k pulse" />
		</>
	);
}

function Platforms() {
	return (
		<>
			{/* Desktop */}
			<rect x="30" y="84" width="200" height="132" rx="8" />
			<path d="M110 216v22M88 238h84" />
			<path d="M84 84v132M100 112h106M100 136h82" />
			<path className="k draw" pathLength={1} d="M100 160h96" />
			<text x="130" y="270">
				Desktop
			</text>
			{/* Web */}
			<rect x="254" y="96" width="124" height="120" rx="8" />
			<path d="M254 116h124" />
			<circle className="dot" cx="266" cy="106" r="2.5" />
			<circle className="dot" cx="276" cy="106" r="2.5" />
			<path d="M286 116v100M298 136h66M298 156h50" />
			<path className="k draw" pathLength={1} d="M298 176h60" />
			<text x="316" y="270">
				Web
			</text>
			{/* Mobile */}
			<rect x="402" y="84" width="56" height="132" rx="10" />
			<path d="M414 108h32M414 128h24" />
			<path className="k draw" pathLength={1} d="M414 148h30" />
			<text x="430" y="270">
				Mobile
			</text>
		</>
	);
}

function Embed() {
	return (
		<>
			<rect x="60" y="50" width="360" height="262" rx="12" />
			<path d="M60 82h360" />
			<circle className="dot" cx="78" cy="66" r="3" />
			<circle className="dot" cx="90" cy="66" r="3" />
			<text x="240" y="71">
				your app
			</text>
			<path d="M80 108h110M80 130h80M80 152h96M80 196h110M80 218h64" />
			<rect className="dash" x="216" y="100" width="186" height="194" rx="8" />
			<g className="e-panel">
				<rect className="k" x="216" y="100" width="186" height="194" rx="8" />
				<path
					className="solid"
					fillRule="evenodd"
					transform="translate(236 121) scale(1.375)"
					d={MARK.small}
				/>
				<text className="strong start" x="266" y="138">
					Architect
				</text>
				<path d="M236 172h146M236 196h110M236 220h130M236 244h84" />
			</g>
		</>
	);
}

function Synced() {
	// Each device and the path its changes take to the node and back.
	const links = [
		"M112 104C170 104 190 150 216 166",
		"M368 104C310 104 290 150 264 166",
		"M240 292V214",
	];
	return (
		<>
			{links.map((d, i) => (
				<g key={d}>
					<path d={d} />
					<circle
						className="flow"
						r="3.5"
						style={
							{
								offsetPath: `path("${d}")`,
								"--d": `${i * 0.9}s`,
							} as React.CSSProperties
						}
					/>
					<circle
						className="flow back"
						r="3.5"
						style={
							{
								offsetPath: `path("${d}")`,
								"--d": `${i * 0.9 + 1.6}s`,
							} as React.CSSProperties
						}
					/>
				</g>
			))}
			{/* The node: two slabs, always on. */}
			<Slab cx={240} cy={186} w={44} t={12} className="k" />
			<Slab cx={240} cy={168} w={44} t={12} className="k" />
			<text className="strong start" x="296" y="200">
				node
			</text>
			{/* Desktop */}
			<rect x="34" y="62" width="78" height="54" rx="6" />
			<path d="M73 116v12M56 128h34" />
			{/* Laptop */}
			<rect x="372" y="66" width="74" height="48" rx="6" />
			<path d="M360 122h98" />
			{/* Phone */}
			<rect x="222" y="292" width="36" height="56" rx="7" />
		</>
	);
}

function Shared() {
	// A vault of six notes; the three in scope are the ones that are shared.
	const notes = [0, 1, 2, 3, 4, 5].map((i) => ({
		x: 44 + (i % 3) * 52,
		y: 96 + Math.floor(i / 3) * 78,
		shared: i === 1 || i === 2 || i === 5,
	}));
	return (
		<>
			{notes.map((n) => (
				<g key={`${n.x}-${n.y}`} className={n.shared ? "k" : undefined}>
					<rect x={n.x} y={n.y} width="40" height="54" rx="5" />
					<path d={`M${n.x + 8} ${n.y + 16}h24M${n.x + 8} ${n.y + 27}h18`} />
				</g>
			))}
			<path
				className="k dash scope"
				d="M88 88h104v70h-52v78h52v-70"
				pathLength={1}
			/>
			<path d="M204 180h54" />
			<circle
				className="flow out"
				r="3.5"
				style={{ offsetPath: 'path("M204 180h54")' } as React.CSSProperties}
			/>
			{/* The published page. */}
			<rect x="266" y="74" width="180" height="212" rx="10" />
			<path d="M266 98h180" />
			<circle className="dot" cx="280" cy="86" r="2.5" />
			<circle className="dot" cx="290" cy="86" r="2.5" />
			{[0, 1, 2].map((i) => (
				<path
					key={i}
					className="k draw"
					pathLength={1}
					style={{ "--d": `${0.6 + i * 0.5}s` } as React.CSSProperties}
					d={`M288 ${130 + i * 44}h${[120, 96, 132][i]}`}
				/>
			))}
			<path d="M288 144h80M288 188h104M288 232h64" />
		</>
	);
}
