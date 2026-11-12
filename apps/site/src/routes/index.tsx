import { createFileRoute } from "@tanstack/react-router";

import { Get } from "#/components/landing/get";
import { Hero } from "#/components/landing/hero";
import { Pillars } from "#/components/landing/pillars";
import { pageHead } from "#/lib/seo";

/**
 * The front door (#559): what Architect is and the download, its six
 * properties as a short list, then every way to get it.
 */
export const Route = createFileRoute("/")({
	head: () => pageHead(),
	component: Home,
});

function Home() {
	return (
		<>
			<Hero />
			<Pillars />
			<Get />
		</>
	);
}
