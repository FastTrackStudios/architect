import { createFileRoute } from "@tanstack/react-router";

import { Feature } from "#/components/landing/feature";
import { Get } from "#/components/landing/get";
import { Hero } from "#/components/landing/hero";
import { FEATURES } from "#/content/landing";
import { pageHead } from "#/lib/seo";

/**
 * The front door (#559): what it is and the download, then the app itself,
 * one heading and one capture at a time, then every way to get it.
 */
export const Route = createFileRoute("/")({
	head: () => pageHead(),
	component: Home,
});

function Home() {
	return (
		<>
			<Hero />
			{FEATURES.map((f) => (
				<Feature key={f.id} feature={f} />
			))}
			<Get />
		</>
	);
}
