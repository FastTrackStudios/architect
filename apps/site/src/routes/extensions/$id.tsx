import { createFileRoute, Link, notFound } from "@tanstack/react-router";

import { Sheet, SheetHead } from "#/components/landing/sheet";
import { Rich } from "#/components/ui/rich";
import { STORE } from "#/content/pages-copy";
import { findExtension, STORE_INDEX } from "#/lib/data";
import { pageHead } from "#/lib/seo";

/** One extension's page: what it is, its versions, and how to install it. */
export const Route = createFileRoute("/extensions/$id")({
	loader: ({ params }) => {
		const ext = findExtension(params.id);
		if (!ext) throw notFound();
		return ext;
	},
	head: ({ loaderData }) =>
		loaderData
			? pageHead({
					title: `${loaderData.name} · ${STORE.title}`,
					description: loaderData.description || STORE.description,
					path: `/extensions/${loaderData.id}`,
				})
			: pageHead({ title: STORE.title }),
	component: ExtensionPage,
});

const fill = (s: string, vars: Record<string, string>) =>
	s.replace(/\{(\w+)\}/g, (_, k: string) => vars[k] ?? `{${k}}`);

function ExtensionPage() {
	const e = Route.useLoaderData();
	const vars = { name: e.name, id: e.id, registry: STORE_INDEX.registry };
	return (
		<Sheet
			id="extension"
			sheet={`A-402 ${e.name}`}
			labelledBy="ext-page-title"
			className="sheet sheet-page"
			strip={{ ...STORE.strip, t: e.name }}
		>
			<div className="sheet-body">
				<p className="page-back">
					<Link className="text-link" to="/extensions">
						{STORE.back}
					</Link>
				</p>
				<SheetHead id="ext-page-title" title={e.name} lede={e.description} />
				{STORE_INDEX.source === "fixture" ? (
					<p className="fixture-note" role="note">
						<Rich text={STORE.fixture} />
					</p>
				) : null}
				<dl className="spec store-spec">
					<div>
						<dt>Id</dt>
						<dd>
							<code>{e.id}</code>
						</dd>
					</div>
					<div>
						<dt>{STORE.columns.version}</dt>
						<dd>{e.version}</dd>
					</div>
					<div>
						<dt>{STORE.columns.kind}</dt>
						<dd>{STORE.kinds[e.kind] ?? e.kind}</dd>
					</div>
					<div>
						<dt>{STORE.columns.publisher}</dt>
						<dd>{e.publisher}</dd>
					</div>
					{e.category ? (
						<div>
							<dt>Category</dt>
							<dd>{e.category}</dd>
						</div>
					) : null}
					{e.platforms.length ? (
						<div>
							<dt>{STORE.platforms}</dt>
							<dd>{e.platforms.join(", ")}</dd>
						</div>
					) : null}
					{e.source.type === "archive" ? (
						<div>
							<dt>{STORE.source}</dt>
							<dd>
								<code>{e.source.archive.split("/").pop()}</code>
								{e.source.sha256 ? (
									<>
										{" "}
										<span className="t-k">sha256</span>{" "}
										<code>{e.source.sha256}</code>
									</>
								) : null}
							</dd>
						</div>
					) : null}
				</dl>
				<div className="notes-block">
					<h3>{STORE.install.title}</h3>
					<p>
						<Rich text={fill(STORE.install.app, vars)} />
					</p>
					<p>
						<Rich text={fill(STORE.install.action, vars)} />
					</p>
				</div>
				<div className="notes-block">
					<h3>{STORE.versions}</h3>
					<ol className="versions">
						{e.versions.map((v) => (
							<li key={v.version}>
								<span className="sn">{v.version}</span> {v.changelog}
							</li>
						))}
					</ol>
				</div>
			</div>
		</Sheet>
	);
}
