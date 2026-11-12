import type { ReactNode } from "react";
import { Rich } from "#/components/ui/rich";

/**
 * A page beside the landing page: its title, its lede,
 * then what it lists.
 */
export function Page({
	title,
	lede,
	before,
	children,
}: {
	title: string;
	lede?: string;
	before?: ReactNode;
	children: ReactNode;
}) {
	return (
		<article className="page wrap">
			{before}
			<header className="page-head">
				<h1>{title}</h1>
				{lede ? (
					<p className="lede">
						<Rich text={lede} />
					</p>
				) : null}
			</header>
			{children}
		</article>
	);
}
