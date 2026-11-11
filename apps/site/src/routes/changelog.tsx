import { createFileRoute } from "@tanstack/react-router";

import { Sheet } from "#/components/landing/sheet";
import { Rich } from "#/components/ui/rich";
import { CHANGELOG } from "#/content/pages-copy";
import { CHANGELOG_RELEASES, day } from "#/lib/data";
import { pageHead } from "#/lib/seo";

/**
 * The last releases' notes, newest first: the GitHub release bodies,
 * which are scripts/release-notes.sh's output, rendered at build time.
 */
export const Route = createFileRoute("/changelog")({
	head: () =>
		pageHead({
			title: CHANGELOG.title,
			description: CHANGELOG.description,
			path: "/changelog",
		}),
	component: Changelog,
});

function Changelog() {
	return (
		<Sheet
			id="changelog"
			sheet="R-001 Revisions"
			labelledBy="cl-title"
			className="sheet sheet-page"
			strip={CHANGELOG.strip}
		>
			<div className="sheet-body">
				<div className="sheet-head">
					<h2 id="cl-title">{CHANGELOG.title}</h2>
					<p className="lede">
						<Rich text={CHANGELOG.lede} />
					</p>
				</div>
				<ol className="releases">
					{CHANGELOG_RELEASES.map((r) => (
						<li key={r.tag} id={r.tag} className="release">
							<header className="release-head">
								<h3>
									<a href={`#${r.tag}`}>{r.tag}</a>
								</h3>
								<span className="release-day">{day(r.publishedAt)}</span>
								{r.prerelease ? (
									<span className="release-pre">{CHANGELOG.prerelease}</span>
								) : null}
								<a className="text-link" href={r.url}>
									{CHANGELOG.onGitHub}
								</a>
							</header>
							<div
								className="prose"
								// biome-ignore lint/security/noDangerouslySetInnerHtml: our own release notes, rendered from markdown at build time (scripts/data.ts)
								dangerouslySetInnerHTML={{ __html: r.html }}
							/>
						</li>
					))}
				</ol>
				<p className="page-links">
					<a className="text-link" href={CHANGELOG.older.href}>
						{CHANGELOG.older.label}
					</a>
				</p>
			</div>
		</Sheet>
	);
}
