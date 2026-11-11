import { createFileRoute, Link } from "@tanstack/react-router";

import { Sheet, SheetHead } from "#/components/landing/sheet";
import { Rich } from "#/components/ui/rich";
import { STORE } from "#/content/pages-copy";
import { STORE_INDEX } from "#/lib/data";
import { pageHead } from "#/lib/seo";

/** The store, browseable: every extension the index lists, at its newest version. */
export const Route = createFileRoute("/extensions/")({
	head: () =>
		pageHead({
			title: STORE.title,
			description: STORE.description,
			path: "/extensions",
		}),
	component: Store,
});

function Store() {
	return (
		<Sheet
			id="store"
			sheet="A-402 Extension store"
			labelledBy="store-title"
			className="sheet sheet-page"
			strip={STORE.strip}
		>
			<div className="sheet-body">
				<SheetHead id="store-title" title={STORE.title} lede={STORE.lede} />
				{STORE_INDEX.source === "fixture" ? (
					<p className="fixture-note" role="note">
						<Rich text={STORE.fixture} />
					</p>
				) : null}
				<ul className="store-list">
					{STORE_INDEX.entries.map((e) => (
						<li key={e.id} className="store-item">
							<h3>
								<Link to="/extensions/$id" params={{ id: e.id }}>
									{e.name}
								</Link>
							</h3>
							<p className="store-desc">{e.description}</p>
							<dl className="spec store-spec">
								<div>
									<dt>{STORE.columns.version}</dt>
									<dd>{e.version}</dd>
								</div>
								<div>
									<dt>{STORE.columns.kind}</dt>
									<dd>{e.kind}</dd>
								</div>
								<div>
									<dt>{STORE.columns.publisher}</dt>
									<dd>{e.publisher}</dd>
								</div>
							</dl>
						</li>
					))}
				</ul>
				<p className="page-links">
					<a className="text-link" href={STORE.list.href}>
						{STORE.list.label}
					</a>
				</p>
			</div>
		</Sheet>
	);
}
