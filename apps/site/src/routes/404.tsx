import { createFileRoute } from "@tanstack/react-router";

import { NotFound } from "#/components/layout/not-found";
import { pageHead } from "#/lib/seo";

/**
 * Prerendered as /404.html, which the web bundle's server serves, with a
 * 404, for any path with no file (scripts/serve.ts). In the app, an
 * unknown route renders the same.
 */
export const Route = createFileRoute("/404")({
	head: () => pageHead({ title: "Not found", path: "/404" }),
	component: NotFound,
});
