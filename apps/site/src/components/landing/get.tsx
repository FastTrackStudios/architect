import { Link } from "@tanstack/react-router";

import { Cmd } from "#/components/ui/cmd";
import { GET } from "#/content/landing";
import { NIX_RUN } from "#/content/site";
import { type Asset, RELEASE } from "#/lib/data";

/** The close: every way to get it, then where to read on. */
export function Get() {
	return (
		<section id={GET.id} className="get wrap" aria-labelledby="get-title">
			<div className="feature-head">
				<h2 id="get-title">{GET.title}</h2>
				<a href={RELEASE.url}>
					{GET.version.replace("{version}", RELEASE.version)}
				</a>
			</div>
			<ul className="get-list">
				<Platform k={GET.linux.k} v={GET.linux.v} asset={RELEASE.linux} />
				<Platform k={GET.macos.k} v={GET.macos.v} asset={RELEASE.macos} />
				<li>
					<h3>{GET.nix.k}</h3>
					<Cmd text={NIX_RUN} copy={GET.copy} copied={GET.copied} />
				</li>
				<li>
					<h3>{GET.web.k}</h3>
					<a className="btn btn-quiet" href={GET.web.href}>
						{GET.web.open}
					</a>
				</li>
			</ul>
			<ul className="get-more">
				{GET.more.map((m) => (
					<li key={m.label}>
						{m.to ? (
							<Link to={m.to}>{m.label}</Link>
						) : (
							<a href={m.href}>{m.label}</a>
						)}
					</li>
				))}
			</ul>
		</section>
	);
}

function Platform({
	k,
	v,
	asset,
}: {
	k: string;
	v: string;
	asset: Asset | null;
}) {
	return (
		<li>
			<h3>
				{k} <span>{asset ? v : GET.missing}</span>
			</h3>
			{asset ? (
				<a className="btn" href={asset.url}>
					{GET.download}
				</a>
			) : null}
		</li>
	);
}
