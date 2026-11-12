import { Link } from "@tanstack/react-router";

import { PILLARS, type Pillar as PillarT } from "#/content/landing";
import { Figure } from "./figures";

/** What Architect is, one at a time: a name, one line, its facts,
 * and a drawing that acts the claim out. */
export function Pillars() {
	return (
		<div className="pillars wrap">
			{PILLARS.map((p) => (
				<Pillar key={p.id} pillar={p} />
			))}
		</div>
	);
}

function Pillar({ pillar: p }: { pillar: PillarT }) {
	return (
		<section id={p.id} className="pillar" aria-labelledby={`${p.id}-title`}>
			<div className="pillar-body">
				<h2 id={`${p.id}-title`}>{p.title}</h2>
				<p className="sub">{p.body}</p>
				{p.status ? (
					<p>
						<a className="tag" href={p.status.href}>
							{p.status.label}
						</a>
					</p>
				) : null}
				{p.chips ? (
					<ul className="chips" aria-label={p.chips.label}>
						{p.chips.items.map((c) => (
							<li key={c.name} className={c.note ? "chip soon" : "chip"}>
								{c.name}
								{c.note ? (
									c.href ? (
										<a href={c.href}>{c.note}</a>
									) : (
										<small>{c.note}</small>
									)
								) : null}
							</li>
						))}
					</ul>
				) : null}
				{p.code ? (
					// biome-ignore lint/a11y/noNoninteractiveTabindex: a code block that scrolls sideways
					<pre className="code" tabIndex={0}>
						<code>{p.code.lines.join("\n")}</code>
					</pre>
				) : null}
				{p.link ? (
					<p>
						{p.link.to ? (
							<Link to={p.link.to}>{p.link.label}</Link>
						) : (
							<a href={p.link.href}>{p.link.label}</a>
						)}
					</p>
				) : null}
			</div>
			<Figure id={p.figure} />
		</section>
	);
}
