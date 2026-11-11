import { Sheet, SheetHead } from "#/components/landing/sheet";
import { NOT_FOUND } from "#/content/pages-copy";

/** No page at this address (apps/web/site/404.html's words). */
export function NotFound() {
	return (
		<Sheet
			id="not-found"
			sheet="404"
			labelledBy="nf-title"
			className="sheet sheet-page"
		>
			<div className="sheet-body">
				<SheetHead
					id="nf-title"
					title={NOT_FOUND.title}
					lede={NOT_FOUND.lede}
				/>
				<p className="page-links">
					{NOT_FOUND.links.map((l) => (
						<a key={l.href} className="text-link" href={l.href}>
							{l.label}
						</a>
					))}
				</p>
			</div>
		</Sheet>
	);
}
