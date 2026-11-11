import { SITE } from "#/content/site";

/**
 * The head tags for one page — title, description, canonical, Open Graph
 * and Twitter — from three facts (the hub's `pageHead()`). Every route
 * builds its head through this so a page can never carry a title without
 * the matching og:title, or a canonical that disagrees with its path.
 *
 * `title` is the page's own; the site name is appended here. Pass nothing
 * for the home page, whose title is the landing page's own.
 */
export function pageHead({
	title,
	description = SITE.description,
	path = "/",
}: {
	title?: string;
	description?: string;
	path?: string;
} = {}) {
	const fullTitle = title ? `${title} · ${SITE.name}` : SITE.homeTitle;
	// The landing page's og:title was the bare name, with its own og:description.
	const ogTitle = title ? fullTitle : SITE.name;
	const ogDescription = title ? description : SITE.ogDescription;
	const url = `${SITE.url}${path === "/" ? "" : path}`;

	return {
		meta: [
			{ title: fullTitle },
			{ name: "description", content: description },
			{ property: "og:title", content: ogTitle },
			{ property: "og:description", content: ogDescription },
			{ property: "og:url", content: url },
			{ name: "twitter:title", content: ogTitle },
			{ name: "twitter:description", content: ogDescription },
		],
		links: [{ rel: "canonical", href: url }],
	};
}
