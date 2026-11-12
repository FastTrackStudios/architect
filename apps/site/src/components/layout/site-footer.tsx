import { SHELL } from "#/content/site";

/** The foot of every page: who made it, the licence, where the source is. */
export function SiteFooter() {
	const f = SHELL.footer;
	return (
		<footer className="site-footer wrap">
			<p>
				{f.by.pre} <a href={f.by.href}>{f.by.org}</a>. {f.tail}
			</p>
			<ul>
				{f.links.map((l) => (
					<li key={l.href}>
						<a href={l.href}>{l.label}</a>
					</li>
				))}
			</ul>
		</footer>
	);
}
