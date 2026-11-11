import { createFileRoute } from "@tanstack/react-router";

import { NotFound } from "#/components/layout/not-found";
import { pageHead } from "#/lib/seo";

/**
 * Prerendered as /404.html, which Vercel serves for any path with no file
 * (scripts/assemble.sh). In the app, an unknown route renders the same.
 */
export const Route = createFileRoute("/404")({
	head: () => pageHead({ title: "Not found", path: "/404" }),
	component: NotFound,
});
