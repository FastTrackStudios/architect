import { createFileRoute } from "@tanstack/react-router";

import { Page } from "#/components/layout/page";
import { Cmd } from "#/components/ui/cmd";
import { Rich } from "#/components/ui/rich";
import { DOWNLOAD } from "#/content/pages-copy";
import { day, RELEASE } from "#/lib/data";
import { formatSize, type ReleaseAsset } from "#/lib/release-assets";
import { useReleaseAssets } from "#/lib/release-check";
import { pageHead } from "#/lib/seo";

/**
 * Per-platform downloads of the release this site was built for. Its
 * links are named by convention from the tag (prerendered, so they work
 * with scripts off); on load, GitHub's API says which files are there,
 * their sizes and sha256 (src/lib/release-assets.ts). A file it does not
 * list is shown as not available yet; if it cannot be asked, the links
 * stay as they are.
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
	const { assets, check } = useReleaseAssets(RELEASE.tag);
	const links = [
		{ asset: assets.sums, label: DOWNLOAD.sums },
		{ asset: assets.sig, label: DOWNLOAD.sig },
		{
			asset: assets.web,
			label: DOWNLOAD.bundle.replace("{file}", assets.web.file),
		},
	].filter((l) => l.asset.state !== "missing");
	return (
		<Page title={DOWNLOAD.title} lede={DOWNLOAD.lede}>
			<p className="label page-meta">
				{DOWNLOAD.release} <a href={RELEASE.url}>{RELEASE.tag}</a>,{" "}
				{day(RELEASE.publishedAt)}
			</p>
			<ul className="rows" data-release-check={check}>
				<Platform
					k={p.linux.k}
					v={p.linux.v}
					note={p.linux.note}
					asset={assets.linux}
					primary
				/>
				<Platform
					k={p.macos.k}
					v={p.macos.v}
					note={p.macos.note}
					asset={assets.macos}
				/>
				<Platform
					k={p.node.k}
					v={p.node.v}
					note={p.node.note}
					asset={assets.node}
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
				<Cmds
					code={DOWNLOAD.nix.code.map((c) => ({
						...c,
						line: c.line.replace("{version}", RELEASE.version),
					}))}
				/>
				<p>
					<a href={DOWNLOAD.nix.more.href}>{DOWNLOAD.nix.more.label}</a>
				</p>
			</section>
			<section className="page-block" aria-labelledby="verify-title">
				<h2 id="verify-title">{DOWNLOAD.verify.title}</h2>
				<p>
					<Rich text={DOWNLOAD.verify.lede} />
				</p>
				<Cmds code={DOWNLOAD.verify.code} />
			</section>
			<p className="page-note">{DOWNLOAD.updates}</p>
			<ul className="page-links">
				{links.map((l) => (
					<li key={l.asset.key}>
						<a href={l.asset.url} data-asset={l.asset.key}>
							{l.label}
						</a>
					</li>
				))}
				<li>
					<a href={RELEASE.url}>{DOWNLOAD.all}</a>
				</li>
			</ul>
		</Page>
	);
}

function Cmds({
	code,
}: {
	code: readonly { comment: string; line: string }[];
}) {
	return (
		<ul className="cmds">
			{code.map((c) => (
				<li key={c.comment}>
					<p className="label">{c.comment}</p>
					<Cmd text={c.line} copy={DOWNLOAD.copy} copied={DOWNLOAD.copied} />
				</li>
			))}
		</ul>
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
	asset: ReleaseAsset;
	primary?: boolean;
}) {
	const there = asset.state !== "missing";
	return (
		<li className="row" data-asset={asset.key} data-state={asset.state}>
			<div className="row-what">
				<h2>{k}</h2>
				<p className="label">{v}</p>
			</div>
			<div className="row-body">
				<p>
					<Rich text={there ? note : DOWNLOAD.missing} />
				</p>
				{there ? (
					<p className="label row-file">
						<code>{asset.file}</code>
						{asset.size !== null ? ` · ${formatSize(asset.size)}` : null}
					</p>
				) : null}
				{there && asset.sha256 ? (
					<p className="label row-sha">
						{DOWNLOAD.sha} <code>{asset.sha256}</code>
					</p>
				) : null}
			</div>
			{there ? (
				<a className={primary ? "btn" : "btn btn-quiet"} href={asset.url}>
					{DOWNLOAD.get}
				</a>
			) : null}
		</li>
	);
}
