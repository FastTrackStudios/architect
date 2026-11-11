import { createFileRoute } from "@tanstack/react-router";
import {
	ExtensionsSheet,
	FilesSheet,
	FrameworkSheet,
	IssueSheet,
	SelfHostSheet,
	StudioSheet,
	SyncSheet,
	VaultSheet,
} from "#/components/landing/sheets";
import { SitePlan } from "#/components/landing/site-plan";
import { pageHead } from "#/lib/seo";

/**
 * The front door: the drawing set, ported from apps/web/landing. A-000 the
 * site plan, then one sheet per subject, then the issue sheet.
 */
export const Route = createFileRoute("/")({
	head: () => pageHead(),
	component: Home,
});

function Home() {
	return (
		<>
			<SitePlan />
			<VaultSheet />
			<FilesSheet />
			<SyncSheet />
			<ExtensionsSheet />
			<StudioSheet />
			<SelfHostSheet />
			<FrameworkSheet />
			<IssueSheet />
		</>
	);
}
