import { NOT_FOUND } from "#/content/pages-copy";
import { Page } from "./page";

/** No page at this address. */
export function NotFound() {
	return (
		<Page title={NOT_FOUND.title} lede={NOT_FOUND.lede}>
			<ul className="page-links">
				{NOT_FOUND.links.map((l) => (
					<li key={l.href}>
						<a href={l.href}>{l.label}</a>
					</li>
				))}
			</ul>
		</Page>
	);
}
