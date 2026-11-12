import { createFileRoute } from "@tanstack/react-router";

import { Page } from "#/components/layout/page";
import { CHANGELOG } from "#/content/pages-copy";
import { CHANGELOG_RELEASES } from "#/lib/changelog";
import { day } from "#/lib/data";
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
		<Page title={CHANGELOG.title} lede={CHANGELOG.lede}>
			<ol className="releases">
				{CHANGELOG_RELEASES.map((r) => (
					<li key={r.tag} id={r.tag} className="release">
						<header className="release-head">
							<h2>
								<a href={`#${r.tag}`}>{r.tag}</a>
							</h2>
							<p className="label">
								{day(r.publishedAt)}
								{r.prerelease ? `, ${CHANGELOG.prerelease}` : null}
							</p>
							<p className="label">
								<a href={r.url}>{CHANGELOG.onGitHub}</a>
							</p>
						</header>
						<div
							className="prose"
							// biome-ignore lint/security/noDangerouslySetInnerHtml: our own release notes, rendered from markdown at build time (scripts/data.ts)
							dangerouslySetInnerHTML={{ __html: r.html }}
						/>
					</li>
				))}
			</ol>
			<ul className="page-links">
				<li>
					<a href={CHANGELOG.older.href}>{CHANGELOG.older.label}</a>
				</li>
			</ul>
		</Page>
	);
}
