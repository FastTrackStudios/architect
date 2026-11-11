import { createFileRoute } from "@tanstack/react-router";

import { Sheet, SheetHead } from "#/components/landing/sheet";
import { Arrow } from "#/components/ui/arrow";
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
		<Sheet
			id="download"
			sheet="I-001 Download"
			labelledBy="dl-title"
			className="sheet sheet-page"
			strip={DOWNLOAD.strip}
		>
			<div className="sheet-body">
				<SheetHead id="dl-title" title={DOWNLOAD.title} lede={DOWNLOAD.lede} />
				<dl className="tb-data release-data">
					<div>
						<dt>{DOWNLOAD.release}</dt>
						<dd>
							<a href={RELEASE.url}>{RELEASE.tag}</a> ·{" "}
							{day(RELEASE.publishedAt)}
						</dd>
					</div>
				</dl>
				<div className="downloads">
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
				</div>
				<div className="notes-block">
					<h3>{DOWNLOAD.nix.title}</h3>
					<p>
						<Rich text={DOWNLOAD.nix.lede} />
					</p>
					{/* biome-ignore lint/a11y/noNoninteractiveTabindex: a code block that scrolls sideways */}
					<pre className="code" tabIndex={0}>
						<code>
							{DOWNLOAD.nix.code
								.map((l) => l.replace("{version}", RELEASE.version))
								.join("\n")}
						</code>
					</pre>
					<p className="notes-link">
						<a className="text-link" href={DOWNLOAD.nix.more.href}>
							{DOWNLOAD.nix.more.label}
						</a>
					</p>
				</div>
				<p className="page-links">
					{RELEASE.sums ? (
						<a className="text-link" href={RELEASE.sums}>
							{DOWNLOAD.sums}
						</a>
					) : null}
					<a className="text-link" href={DOWNLOAD.all.href}>
						{DOWNLOAD.all.label}
					</a>
				</p>
			</div>
		</Sheet>
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
		<div className="platform">
			{asset ? (
				<a className={primary ? "act act-primary" : "act"} href={asset.url}>
					<span className="act-k">{k}</span>
					<span className="act-v">
						{v} · {asset.file}
					</span>
					<Arrow dir="down" />
				</a>
			) : (
				<div className="act act-none">
					<span className="act-k">{k}</span>
					<span className="act-v">{DOWNLOAD.missing}</span>
				</div>
			)}
			<p className="platform-note">
				<Rich text={note} />
			</p>
			{asset?.sha256 ? (
				<p className="platform-sha">
					<span className="t-k">{DOWNLOAD.sha}</span>{" "}
					<code>{asset.sha256}</code>
				</p>
			) : null}
		</div>
	);
}
