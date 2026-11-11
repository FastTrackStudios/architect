import { FILES } from "#/content/landing";

/** Section A through one file: its three holders and the policy it meets. */
export function SectionCut() {
	return (
		<figure className="section-cut" aria-labelledby="cut-cap">
			{/* biome-ignore lint/a11y/useSemanticElements: the landing page's markup, ported as is */}
			<div
				className="cut-scroll"
				// biome-ignore lint/a11y/noNoninteractiveTabindex: it scrolls sideways; the keyboard must reach it
				tabIndex={0}
				role="group"
				aria-label="Section A, scrolls sideways on small screens"
			>
				<svg
					viewBox="0 0 900 300"
					role="img"
					aria-labelledby="cut-title cut-desc"
				>
					<title id="cut-title">Section A through one file</title>
					<desc id="cut-desc">
						One audio file, 149-BASS DI.wav, 705.6 KB, cut through its holders.
						The node holds it in full, verified one second ago. The desk holds a
						cached copy, verified three seconds ago. The laptop holds it in full
						but is offline, unverified for two minutes. Policy asks for two
						verified copies; the file has two.
					</desc>
					<text className="t-label t-dim" x="20" y="28">
						SECTION A · 149-BASS DI-241011_2001.WAV · 705.6 KB
					</text>
					<line className="ground" x1="20" y1="244" x2="880" y2="244" />
					<rect
						className="ground-hatch"
						x="20"
						y="244"
						width="860"
						height="22"
						fill="url(#cut-hatch)"
					/>
					<defs>
						<pattern
							id="cut-hatch"
							width="8"
							height="8"
							patternUnits="userSpaceOnUse"
							patternTransform="rotate(45)"
						>
							<line x1="0" y1="0" x2="0" y2="8" className="hatch-line" />
						</pattern>
					</defs>

					<g className="holder">
						<rect className="wall" x="40" y="70" width="230" height="174" />
						<text className="t-room" x="58" y="102">
							NODE
						</text>
						<text className="t-label" x="58" y="124">
							starcommand · full
						</text>
						<rect
							className="copy copy-ok"
							x="58"
							y="150"
							width="194"
							height="40"
						/>
						<text className="t-label t-on" x="70" y="175">
							VERIFIED 1 s AGO
						</text>
						<text className="t-label" x="58" y="222">
							reachable now
						</text>
					</g>
					<g className="holder">
						<rect className="wall" x="335" y="70" width="230" height="174" />
						<text className="t-room" x="353" y="102">
							DESK
						</text>
						<text className="t-label" x="353" y="124">
							studio · cached
						</text>
						<rect
							className="copy copy-ok"
							x="353"
							y="150"
							width="194"
							height="40"
						/>
						<text className="t-label t-on" x="365" y="175">
							VERIFIED 3 s AGO
						</text>
						<text className="t-label" x="353" y="222">
							this device
						</text>
					</g>
					<g className="holder holder-off">
						<rect className="wall" x="630" y="70" width="230" height="174" />
						<text className="t-room" x="648" y="102">
							LAPTOP
						</text>
						<text className="t-label" x="648" y="124">
							home · full
						</text>
						<rect
							className="copy copy-un"
							x="648"
							y="150"
							width="194"
							height="40"
						/>
						<text className="t-label" x="660" y="175">
							UNVERIFIED · 2 min
						</text>
						<path
							className="cloud"
							d="M640 146 q8 -12 16 0 q8 -12 16 0 q8 -12 16 0 q8 -12 16 0 q8 -12 16 0 q8 -12 16 0 q8 -12 16 0 q8 -12 16 0 q8 -12 16 0 q8 -12 16 0 q8 -12 16 0 q8 -12 16 0 q12 8 0 16 q12 8 0 16 q12 8 0 16 q-8 12 -16 0 q-8 12 -16 0 q-8 12 -16 0 q-8 12 -16 0 q-8 12 -16 0 q-8 12 -16 0 q-8 12 -16 0 q-8 12 -16 0 q-8 12 -16 0 q-8 12 -16 0 q-8 12 -16 0 q-8 12 -16 0 q-12 -8 0 -16 q-12 -8 0 -16 q-12 -8 0 -16 Z"
						/>
						<g className="cloud-tag" transform="translate(842 140)">
							<path d="M0 -13 L13 9 L-13 9 Z" />
							<text y="5" textAnchor="middle">
								1
							</text>
						</g>
						<text className="t-label" x="648" y="222">
							offline
						</text>
					</g>
					<g className="dim">
						<line
							x1="40"
							y1="282"
							x2="565"
							y2="282"
							markerStart="url(#tick)"
							markerEnd="url(#tick)"
						/>
						<text className="t-dimtext" x="302" y="276" textAnchor="middle">
							2 VERIFIED COPIES · POLICY 2 · MET
						</text>
					</g>
				</svg>
			</div>
			<figcaption id="cut-cap" className="cut-cap">
				{FILES.cutCaption}
			</figcaption>
		</figure>
	);
}
