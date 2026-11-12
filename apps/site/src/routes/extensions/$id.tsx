import { createFileRoute, Link, notFound } from "@tanstack/react-router";

import { Page } from "#/components/layout/page";
import { Rich } from "#/components/ui/rich";
import { STORE } from "#/content/pages-copy";
import { pageHead } from "#/lib/seo";
import { findExtension, STORE_INDEX } from "#/lib/store";

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
		<Page
			title={e.name}
			lede={e.description}
			before={
				<p className="page-back">
					<Link to="/extensions">{STORE.back}</Link>
				</p>
			}
		>
			{STORE_INDEX.source === "fixture" ? (
				<p className="page-note unlit" role="note">
					<Rich text={STORE.fixture} />
				</p>
			) : null}
			<dl className="spec">
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
									sha256 <code>{e.source.sha256}</code>
								</>
							) : null}
						</dd>
					</div>
				) : null}
			</dl>
			<section className="page-block" aria-labelledby="install-title">
				<h2 id="install-title">{STORE.install.title}</h2>
				<p>
					<Rich text={fill(STORE.install.app, vars)} />
				</p>
				<p>
					<Rich text={fill(STORE.install.action, vars)} />
				</p>
			</section>
			<section className="page-block" aria-labelledby="versions-title">
				<h2 id="versions-title">{STORE.versions}</h2>
				<ol className="versions">
					{e.versions.map((v) => (
						<li key={v.version}>
							<span className="label">{v.version}</span> {v.changelog}
						</li>
					))}
				</ol>
			</section>
		</Page>
	);
}
