import { COLOPHON } from "#/content/landing";

/** The revisions table and the colophon line at the foot of every page. */
export function Colophon() {
	return (
		<footer className="colophon">
			<table className="revisions">
				<caption>{COLOPHON.caption}</caption>
				<thead>
					<tr>
						{COLOPHON.columns.map((c) => (
							<th key={c} scope="col">
								{c}
							</th>
						))}
					</tr>
				</thead>
				<tbody>
					{COLOPHON.rows.map((r) => (
						<tr key={r.rev}>
							<td>{r.rev}</td>
							<td>
								<a href={r.href}>{r.label}</a>
							</td>
							<td>{r.by}</td>
						</tr>
					))}
				</tbody>
			</table>
			<p className="colophon-line">
				{COLOPHON.byline.pre}{" "}
				<a href={COLOPHON.byline.href}>{COLOPHON.byline.org}</a>
				{COLOPHON.tail}
			</p>
		</footer>
	);
}
