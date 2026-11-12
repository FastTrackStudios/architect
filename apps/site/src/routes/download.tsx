import { createFileRoute } from "@tanstack/react-router";

import { Page } from "#/components/layout/page";
import { Cmd } from "#/components/ui/cmd";
import { Rich } from "#/components/ui/rich";
import { DOWNLOAD } from "#/content/pages-copy";
import { type Asset, day, RELEASE } from "#/lib/data";
import { pageHead } from "#/lib/seo";

/**
 * Per-platform downloads from the latest release's updater manifests
 * (latest-linux.json, latest-macos.json), read at build time.
 */
export const Route = createFileRoute("/download")({
	head: () =>
		pageHead({
			title: DOWNLOAD.title,
			description: DOWNLOAD.description,
			path: "/download",
		}),
	component: Download,
});

function Download() {
	const p = DOWNLOAD.platforms;
	return (
		<Page title={DOWNLOAD.title} lede={DOWNLOAD.lede}>
			<p className="label page-meta">
				{DOWNLOAD.release} <a href={RELEASE.url}>{RELEASE.tag}</a>,{" "}
				{day(RELEASE.publishedAt)}
			</p>
			<ul className="rows">
				<Platform
					k={p.linux.k}
					v={p.linux.v}
					note={p.linux.note}
					asset={RELEASE.linux}
					primary
				/>
				<Platform
					k={p.macos.k}
					v={p.macos.v}
					note={p.macos.note}
					asset={RELEASE.macos}
				/>
				<Platform
					k={p.node.k}
					v={p.node.v}
					note={p.node.note}
					asset={
						RELEASE.node && {
							...RELEASE.node,
							version: RELEASE.version,
							sha256: "",
						}
					}
				/>
				<li className="row">
					<div className="row-what">
						<h2>{DOWNLOAD.web.k}</h2>
						<p className="label">{DOWNLOAD.web.v}</p>
					</div>
					<div className="row-body">
						<p>{DOWNLOAD.web.note}</p>
					</div>
					<a className="btn btn-quiet" href={DOWNLOAD.web.href}>
						{DOWNLOAD.web.open}
					</a>
				</li>
			</ul>
			<section className="page-block" aria-labelledby="nix-title">
				<h2 id="nix-title">{DOWNLOAD.nix.title}</h2>
				<p>
					<Rich text={DOWNLOAD.nix.lede} />
				</p>
				<ul className="cmds">
					{DOWNLOAD.nix.code.map((c) => (
						<li key={c.comment}>
							<p className="label">{c.comment}</p>
							<Cmd
								text={c.line.replace("{version}", RELEASE.version)}
								copy={DOWNLOAD.copy}
								copied={DOWNLOAD.copied}
							/>
						</li>
					))}
				</ul>
				<p>
					<a href={DOWNLOAD.nix.more.href}>{DOWNLOAD.nix.more.label}</a>
				</p>
			</section>
			<p className="page-note">{DOWNLOAD.updates}</p>
			<ul className="page-links">
				{RELEASE.sums ? (
					<li>
						<a href={RELEASE.sums}>{DOWNLOAD.sums}</a>
					</li>
				) : null}
				<li>
					<a href={DOWNLOAD.all.href}>{DOWNLOAD.all.label}</a>
				</li>
			</ul>
		</Page>
	);
}

function Platform({
	k,
	v,
	note,
	asset,
	primary,
}: {
	k: string;
	v: string;
	note: string;
	asset: Asset | null;
	primary?: boolean;
}) {
	return (
		<li className="row">
			<div className="row-what">
				<h2>{k}</h2>
				<p className="label">{v}</p>
			</div>
			<div className="row-body">
				<p>
					<Rich text={asset ? note : DOWNLOAD.missing} />
				</p>
				{asset ? (
					<p className="label row-file">
						<code>{asset.file}</code>
					</p>
				) : null}
				{asset?.sha256 ? (
					<p className="label row-sha">
						{DOWNLOAD.sha} <code>{asset.sha256}</code>
					</p>
				) : null}
			</div>
			{asset ? (
				<a className={primary ? "btn" : "btn btn-quiet"} href={asset.url}>
					{DOWNLOAD.get}
				</a>
			) : null}
		</li>
	);
}
