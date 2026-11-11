import type { Action } from "#/content/landing";
import { Arrow } from "./arrow";

/** A column of actions: the landing page's "Get Architect" group. */
export function Actions({
	actions,
	label,
	className = "tb-actions",
}: {
	actions: readonly Action[];
	label: string;
	className?: string;
}) {
	return (
		// biome-ignore lint/a11y/useSemanticElements: the landing page's markup, ported as is
		<div className={className} role="group" aria-label={label}>
			{actions.map((a) => (
				<a
					key={a.k}
					className={a.primary ? "act act-primary" : "act"}
					href={a.href}
				>
					<span className="act-k">{a.k}</span>
					{a.v ? <span className="act-v">{a.v}</span> : null}
					<Arrow dir={a.arrow} />
				</a>
			))}
		</div>
	);
}
