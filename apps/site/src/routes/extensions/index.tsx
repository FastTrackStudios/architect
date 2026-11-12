import { createFileRoute, Link } from "@tanstack/react-router";

import { Page } from "#/components/layout/page";
import { Rich } from "#/components/ui/rich";
import { STORE } from "#/content/pages-copy";
import { pageHead } from "#/lib/seo";
import { STORE_INDEX } from "#/lib/store";

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
		<Page title={STORE.title} lede={STORE.lede}>
			{STORE_INDEX.source === "fixture" ? (
				<p className="page-note unlit" role="note">
					<Rich text={STORE.fixture} />
				</p>
			) : null}
			<ul className="rows">
				{STORE_INDEX.entries.map((e) => (
					<li key={e.id} className="row">
						<div className="row-what">
							<h2>
								<Link to="/extensions/$id" params={{ id: e.id }}>
									{e.name}
								</Link>
							</h2>
							<p className="label">{e.publisher}</p>
						</div>
						<div className="row-body">
							<p>{e.description}</p>
						</div>
						<dl className="row-spec label">
							<div>
								<dt>{STORE.columns.version}</dt>
								<dd>{e.version}</dd>
							</div>
							<div>
								<dt>{STORE.columns.kind}</dt>
								<dd>{e.kind}</dd>
							</div>
						</dl>
					</li>
				))}
			</ul>
			<ul className="page-links">
				<li>
					<a href={STORE.list.href}>{STORE.list.label}</a>
				</li>
			</ul>
		</Page>
	);
}
