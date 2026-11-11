import { Rich } from "#/components/ui/rich";
import type { Keynote } from "#/content/landing";

/** A numbered list of keynotes: the hexagon number, a bold lead, the note. */
export function Keynotes({
	notes,
	label,
	className = "keynotes",
}: {
	notes: readonly Keynote[];
	label: string;
	className?: string;
}) {
	return (
		<ol className={className} aria-label={label}>
			{notes.map((k, i) => (
				<li key={k.title}>
					<span className="kn">{i + 1}</span>
					<p>
						<strong>{k.title}</strong> <Rich text={k.body} />
					</p>
				</li>
			))}
		</ol>
	);
}
