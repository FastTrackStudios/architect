import { type ReactNode, useEffect, useRef, useState } from "react";

import { Actions } from "#/components/ui/actions";
import { Rich } from "#/components/ui/rich";
import { GET_ACTIONS, HERO, type Room, SHEETS } from "#/content/landing";
import { Sheet } from "./sheet";

/** Which legs of the sync line (the .sync paths, in order) each room lights. */
const LEGS: Record<Room, number[]> = {
	desk: [0],
	node: [0, 1, 2],
	phone: [1],
	web: [2],
	laptop: [3],
};

/**
 * A-000, the site plan: the plot of one person's devices, its keynotes and
 * the title block. Pointing at (or focusing) a room lights it, its keynote
 * and its leg of the sync line together; on a wide screen a click on a
 * room lights it instead of jumping to its keynote.
 */
export function SitePlan() {
	const [active, setActive] = useState<Room | null>(null);
	const plan = useRef<SVGSVGElement>(null);
	const [drawn, setDrawn] = useState(false);

	useEffect(() => {
		const id = requestAnimationFrame(() => setDrawn(true));
		// On a phone the plan scrolls sideways: open it centred on the node.
		const scroller = plan.current?.parentElement;
		if (scroller && scroller.scrollWidth > scroller.clientWidth) {
			scroller.scrollLeft = (scroller.scrollWidth - scroller.clientWidth) / 2;
		}
		return () => cancelAnimationFrame(id);
	}, []);

	const hover = (room: Room) => ({
		onPointerEnter: () => setActive(room),
		onPointerLeave: () => setActive(null),
		onFocus: () => setActive(room),
		onBlur: () => setActive(null),
	});
	const on = (room: Room) => (active === room ? " is-active" : "");
	const leg = (i: number) =>
		active && LEGS[active].includes(i) ? " is-active" : "";

	const room = (r: Room, className: string, children: ReactNode) => (
		<a
			className={`${className}${on(r)}`}
			href={`#kn-${r}`}
			data-room={r}
			aria-describedby={`kn-${r}`}
			{...hover(r)}
			onClick={(e) => {
				if (window.matchMedia("(min-width: 1241px)").matches) {
					e.preventDefault();
					setActive(r);
				}
			}}
		>
			{children}
		</a>
	);

	return (
		<Sheet
			id="site-plan"
			sheet="A-000 Site plan"
			labelledBy="hero-title"
			className="sheet sheet-hero"
		>
			<div className="hero-plan">
				<figure className="plan" aria-labelledby="plan-cap">
					{/* biome-ignore lint/a11y/useSemanticElements: the landing page's markup, ported as is */}
					<div
						className="plan-scroll"
						// biome-ignore lint/a11y/noNoninteractiveTabindex: it scrolls sideways; the keyboard must reach it
						tabIndex={0}
						role="group"
						aria-label="Site plan, scrolls sideways on small screens"
					>
						<svg
							ref={plan}
							className={drawn ? "plan-svg is-drawn" : "plan-svg"}
							viewBox="0 0 760 540"
							role="img"
							aria-labelledby="plan-title plan-desc"
						>
							<title id="plan-title">Site plan of one person's devices</title>
							<desc id="plan-desc">
								Five rooms on one plot: Desk, Laptop, Phone, Web and the Node.
								One sync line joins them through the Node, which is always on.
								The Laptop is offline, drawn dashed, and keeps working on its
								own copy.
							</desc>
							<defs>
								<pattern
									id="hatch"
									width="8"
									height="8"
									patternUnits="userSpaceOnUse"
									patternTransform="rotate(45)"
								>
									<line x1="0" y1="0" x2="0" y2="8" className="hatch-line" />
								</pattern>
								<marker
									id="tick"
									viewBox="0 0 10 10"
									refX="5"
									refY="5"
									markerWidth="9"
									markerHeight="9"
									orient="auto"
								>
									<line x1="1" y1="9" x2="9" y2="1" className="tick-line" />
								</marker>
							</defs>

							{/* property line */}
							<rect
								className="property"
								x="16"
								y="16"
								width="728"
								height="508"
							/>
							<text className="t-label t-dim" x="28" y="36">
								PROPERTY LINE · ONE ACCOUNT
							</text>
							{/* north arrow */}
							<g className="north" transform="translate(700 58)">
								<circle r="16" />
								<path d="M0 -14 L6 8 L0 3 L-6 8 Z" />
								<text className="t-label" y="34" textAnchor="middle">
									N
								</text>
							</g>

							{/* the sync line: solid where peers are online */}
							<path
								className={`sync sync-main${leg(0)}`}
								pathLength={1}
								d="M232 128 H380 V214"
							/>
							<path
								className={`sync sync-main${leg(1)}`}
								pathLength={1}
								d="M380 346 V430 H196"
							/>
							<path
								className={`sync sync-main${leg(2)}`}
								pathLength={1}
								d="M466 280 H628 V376"
							/>
							<path className={`sync sync-off${leg(3)}`} d="M628 280 V186" />

							{/* rooms */}
							{room(
								"node",
								"room",
								<>
									<rect
										className="wall"
										x="294"
										y="214"
										width="172"
										height="132"
									/>
									<rect
										className="room-fill"
										x="300"
										y="220"
										width="160"
										height="120"
									/>
									<text className="t-room" x="314" y="250">
										NODE
									</text>
									<text className="t-label" x="314" y="272">
										always on · headless
									</text>
									<text className="t-num" x="314" y="306">
										27/27
									</text>
									<text className="t-label" x="314" y="326">
										files held in full
									</text>
									<KnMark x={440} y={238} n={1} />
								</>,
							)}
							{room(
								"desk",
								"room",
								<>
									<rect
										className="wall"
										x="56"
										y="66"
										width="176"
										height="124"
									/>
									<rect
										className="room-fill"
										x="62"
										y="72"
										width="164"
										height="112"
									/>
									<text className="t-room" x="76" y="102">
										DESK
									</text>
									<text className="t-label" x="76" y="124">
										studio · pinned
									</text>
									<text className="t-num" x="76" y="160">
										2
									</text>
									<text className="t-label" x="98" y="160">
										verified copies
									</text>
									<KnMark x={206} y={90} n={2} />
								</>,
							)}
							{room(
								"laptop",
								"room room-off",
								<>
									<rect
										className="wall"
										x="540"
										y="66"
										width="176"
										height="120"
									/>
									<rect
										className="room-fill"
										x="546"
										y="72"
										width="164"
										height="108"
									/>
									<text className="t-room" x="560" y="102">
										LAPTOP
									</text>
									<text className="t-label" x="560" y="124">
										home · offline
									</text>
									<text className="t-label" x="560" y="156">
										keeps working on
									</text>
									<text className="t-label" x="560" y="172">
										its own copy
									</text>
									<KnMark x={690} y={90} n={3} />
								</>,
							)}
							{room(
								"phone",
								"room",
								<>
									<rect
										className="wall"
										x="56"
										y="376"
										width="140"
										height="124"
									/>
									<rect
										className="room-fill"
										x="62"
										y="382"
										width="128"
										height="112"
									/>
									<text className="t-room" x="76" y="412">
										PHONE
									</text>
									<text className="t-label" x="76" y="434">
										same vault
									</text>
									<text className="t-label" x="76" y="452">
										same tabs
									</text>
									<KnMark x={170} y={400} n={4} />
								</>,
							)}
							{room(
								"web",
								"room",
								<>
									<rect
										className="wall"
										x="540"
										y="376"
										width="176"
										height="124"
									/>
									<rect
										className="room-fill"
										x="546"
										y="382"
										width="164"
										height="112"
									/>
									<text className="t-room" x="560" y="412">
										WEB
									</text>
									<text className="t-label" x="560" y="434">
										/app/ in a browser
									</text>
									<text className="t-label" x="560" y="452">
										signed in, live
									</text>
									<KnMark x={690} y={400} n={5} />
								</>,
							)}

							{/* dimension: edit to peers */}
							<g className="dim">
								<line
									x1="232"
									y1="104"
									x2="380"
									y2="104"
									markerStart="url(#tick)"
									markerEnd="url(#tick)"
								/>
								<text className="t-dimtext" x="306" y="96" textAnchor="middle">
									AN EDIT · WITHIN 1 s
								</text>
							</g>
							{/* section cut marker */}
							<g className="cut" transform="translate(500 300)">
								<circle r="15" />
								<line x1="-15" y1="0" x2="15" y2="0" />
								<text className="t-label" y="-3" textAnchor="middle">
									A
								</text>
								<text className="t-label" y="11" textAnchor="middle">
									201
								</text>
							</g>
						</svg>
					</div>
					<p className="plan-more" aria-hidden="true">
						{HERO.planMore.map((s) => (
							<span key={s}>{s}</span>
						))}
					</p>
					<figcaption id="plan-cap" className="plan-cap">
						<span className="t-label">{HERO.planCaptionLabel}</span>,{" "}
						{HERO.planCaption}
					</figcaption>
				</figure>

				<ol className="keynotes hero-keynotes" aria-label="Keynotes">
					{HERO.keynotes.map((k, i) => (
						<li
							key={k.room}
							id={`kn-${k.room}`}
							data-room={k.room}
							className={on(k.room).trim() || undefined}
							{...hover(k.room)}
						>
							<span className="kn">{i + 1}</span>
							<p>
								<strong>{k.title}</strong> <Rich text={k.body} />
							</p>
						</li>
					))}
				</ol>
			</div>

			<aside className="title-block" aria-label="Title block">
				<div className="tb-row tb-project">
					<h1 id="hero-title">
						<span className="h1-word">{HERO.title}</span>
					</h1>
				</div>
				<p className="tb-lede">{HERO.lede}</p>
				<Actions actions={GET_ACTIONS} label="Get Architect" />
				<dl className="tb-data">
					{HERO.data.map((d) => (
						<div key={d.dt}>
							<dt>{d.dt}</dt>
							<dd>{d.dd}</dd>
						</div>
					))}
				</dl>
				<nav className="tb-index" aria-label="Sheet index">
					<span className="tb-k">{HERO.indexLabel}</span>
					<ol>
						{SHEETS.map((s) => (
							<li key={s.id}>
								<a href={`#${s.id}`}>
									<span className="sn">{s.sn}</span> {s.title}
								</a>
							</li>
						))}
					</ol>
				</nav>
				<div className="tb-sheet">
					<span className="tb-k">{HERO.sheet.k}</span>
					<span className="tb-sn">{HERO.sheet.sn}</span>
					<span className="tb-st">{HERO.sheet.title}</span>
				</div>
			</aside>
		</Sheet>
	);
}

/** A keynote's number on the plan, in the hexagon (#hex, in the page's defs). */
function KnMark({ x, y, n }: { x: number; y: number; n: number }) {
	return (
		<g className="kn-mark" transform={`translate(${x} ${y})`}>
			<use href="#hex" x="-12" y="-11" width="24" height="22" />
			<text>{n}</text>
		</g>
	);
}
