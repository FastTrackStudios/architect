import { Link } from "@tanstack/react-router";

import { Shot } from "#/components/ui/shot";
import type { Feature as FeatureT } from "#/content/landing";

/** One heading over one capture, or over two side by side. */
export function Feature({ feature: f }: { feature: FeatureT }) {
	return (
		<section
			id={f.id}
			className="feature wrap"
			aria-labelledby={`${f.id}-title`}
		>
			<div className="feature-head">
				<h2 id={`${f.id}-title`}>{f.title}</h2>
				{f.link ? (
					f.link.to ? (
						<Link to={f.link.to}>{f.link.label}</Link>
					) : (
						<a href={f.link.href}>{f.link.label}</a>
					)
				) : null}
			</div>
			{f.list ? (
				<ul className="feature-list" aria-label={f.list.label}>
					{f.list.items.map((i) => (
						<li key={i.name}>
							{i.name}
							{i.note ? <small> {i.note}</small> : null}
						</li>
					))}
				</ul>
			) : null}
			<div
				className={f.media.length > 1 ? "feature-media pair" : "feature-media"}
			>
				{f.media.map((m) => (
					<div key={m.src} className="frame">
						<Shot
							media={m}
							sizes={
								f.media.length > 1
									? "(max-width: 60rem) 100vw, 40rem"
									: undefined
							}
						/>
					</div>
				))}
			</div>
		</section>
	);
}
