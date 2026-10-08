import type { AgencyPerformanceProfile } from '../../../types';
import ChartFrame from '../frame/ChartFrame';
import { Axis } from '../frame/Axis';
import { ReferenceBand, ReferenceLine } from '../frame/ReferenceLine';
import { TooltipContent } from '../frame/Tooltip';
import { formatValue } from '../core/format';
import { measureText } from '../core/layout';
import { scaleLinear } from '../core/scale';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { Beeswarm, Dot } from '../marks/Dot';

/*
 * F1b PeerStrip: one agency's average closure time among its portal's peers, on one days axis.
 * The typical band is the peer mean ± 2 sd (the same z ≥ 2 the engine flags). A z ≥ 2 agency gets
 * an ink ring and the word 'statistical'; nothing here is crimson and there is no statutory line,
 * because an average says nothing about any one request's deadline.
 */

/** Matches the engine's peer latency outlier threshold. */
const OUTLIER_Z = 2;
const SMALL_SAMPLE = 5;
const CY = 52;
const HEIGHT = 96;

function days(value: number): string {
  return formatValue(value, { kind: 'days' });
}

export interface PeerStripModel {
  focus: AgencyPerformanceProfile;
  peers: AgencyPerformanceProfile[];
  unmeasured: AgencyPerformanceProfile[];
  mean: number | null;
  sd: number | null;
  outlier: boolean;
  headline: string;
}

export function peerStripModel(focus: AgencyPerformanceProfile, all: readonly AgencyPerformanceProfile[]): PeerStripModel {
  const portal = all.filter((profile) => profile.portalSubdomain === focus.portalSubdomain);
  const peers = portal.filter((profile) => profile.agencyId !== focus.agencyId && profile.averageClosureDays !== null);
  const unmeasured = portal.filter((profile) => profile.agencyId !== focus.agencyId && profile.averageClosureDays === null);
  const mean = focus.globalMeanClosureDays;
  const sd = focus.globalStddevClosureDays;
  const outlier = focus.performanceZScore !== null && focus.performanceZScore >= OUTLIER_Z;
  let headline: string;
  if (focus.averageClosureDays === null) {
    headline = `${focus.agencyName} has no measurable closure, so it is not placed among its ${peers.length} measured peers.`;
  } else {
    const relation = mean === null ? '' : focus.averageClosureDays > mean ? `, above the peer mean of ${days(mean)}` : `, at or below the peer mean of ${days(mean)}`;
    headline = `Average closure ${days(focus.averageClosureDays)} across ${focus.validLatencySampleSize.toLocaleString('en-US')} measured closures${relation}.${outlier ? ' Slower than peers (statistical; a comparison, not a statutory finding).' : ''}`;
  }
  return { focus, peers, unmeasured, mean, sd, outlier, headline };
}

function StripPlot({ model, width, onOpenPeer }: { model: PeerStripModel; width: number; onOpenPeer(profile: AgencyPerformanceProfile): void }) {
  const tooltip = useTooltip();
  const { focus, peers, mean, sd } = model;
  const values = [...peers.map((peer) => peer.averageClosureDays as number), focus.averageClosureDays ?? 0];
  if (mean !== null && sd !== null) values.push(mean + 2 * sd);
  const x = scaleLinear([0, Math.max(1, ...values)], [16, width - 16]);
  const ordered = [...peers].sort((a, b) => (a.averageClosureDays as number) - (b.averageClosureDays as number) || a.agencyName.localeCompare(b.agencyName));
  const roving = useRovingFocus({ shape: { rows: Math.max(1, ordered.length) }, onActivate: (i) => ordered[i] && onOpenPeer(ordered[i]), onEscape: tooltip.hide });
  const peerName = (peer: AgencyPerformanceProfile) => `${peer.agencyName}: ${days(peer.averageClosureDays as number)} average closure (n=${peer.validLatencySampleSize})${peer.validLatencySampleSize < SMALL_SAMPLE ? ', small sample' : ''}${peer.performanceZScore !== null ? `, ${formatValue(peer.performanceZScore, { kind: 'z' })}` : ''}. Opens its inspector layer.`;
  const focusLabel = focus.averageClosureDays === null ? null : `${days(focus.averageClosureDays)} (n=${focus.validLatencySampleSize})`;
  const focusX = focus.averageClosureDays === null ? null : x(focus.averageClosureDays);
  const labelSide = focusX !== null && focusLabel && focusX + measureText(focusLabel) + 16 > width ? 'start' : 'end';
  return (
    <g>
      {mean !== null && sd !== null && sd > 0 && (
        <ReferenceBand axis="x" from={x(Math.max(0, mean - 2 * sd))} to={x(mean + 2 * sd)} extent={[16, CY + 22]} label="typical range (within 2 sd)" />
      )}
      {mean !== null && <ReferenceLine kind="peer-mean" axis="x" at={x(mean)} extent={[4, CY + 22]} label={`peer mean ${days(mean)}`} labelSide={x(mean) > width / 2 ? 'start' : 'end'} />}
      <Beeswarm
        cy={CY}
        x={x}
        items={ordered.map((peer, index) => {
          const name = peerName(peer);
          const tip = <TooltipContent title={peer.agencyName} rows={[{ key: 'd', value: days(peer.averageClosureDays as number), label: `average closure, n=${peer.validLatencySampleSize}` }, ...(peer.performanceZScore !== null ? [{ key: 'z', value: formatValue(peer.performanceZScore, { kind: 'z' }), label: 'against the portal mean' }] : [])]} />;
          return {
            key: `${peer.portalSubdomain}:${peer.agencyId}`,
            value: peer.averageClosureDays as number,
            role: 'comparison' as const,
            hollow: true,
            title: name,
            interaction: {
              name,
              item: roving.getItemProps(index),
              onPointerMove: (event) => tooltip.show(tip, { clientX: event.clientX, clientY: event.clientY }),
              onPointerLeave: tooltip.hide,
              onFocusMark: (box) => tooltip.show(tip, box),
              onBlurMark: tooltip.hide,
              onClick: () => onOpenPeer(peer),
            },
          };
        })}
      />
      {focusX !== null && model.outlier && <Dot cx={focusX} cy={CY} r={9} shape="ring" claim="statistical" />}
      {focusX !== null && focusLabel && (
        <Dot cx={focusX} cy={CY} r={5.5} role="primary" label={focusLabel} labelSide={labelSide} title={`${focus.agencyName}: ${focusLabel}`} />
      )}
      {model.outlier && focusX !== null && (
        <text x={labelSide === 'end' ? focusX + 14 : focusX - 14} y={CY + 16} textAnchor={labelSide === 'end' ? 'start' : 'end'} className="fill-fg-secondary text-[11px]">slower than peers (statistical)</text>
      )}
      <Axis scale={x} kind="days" orient="bottom" offset={CY + 24} />
      {tooltip.node}
    </g>
  );
}

export default function PeerStrip({ model, onOpenPeer }: { model: PeerStripModel; onOpenPeer(profile: AgencyPerformanceProfile): void }) {
  const { focus, peers, unmeasured } = model;
  const rows = [focus, ...peers, ...unmeasured].map((profile) => ({
    agency: profile.agencyId === focus.agencyId ? `${profile.agencyName} (this agency)` : profile.agencyName,
    days: profile.averageClosureDays ?? { state: 'not-recorded' as const, reason: 'no measurable closure' },
    n: profile.validLatencySampleSize,
    z: profile.performanceZScore ?? { state: 'not-recorded' as const, reason: 'no closure sample or no spread' },
  }));
  return (
    <div className="grid min-w-0 gap-1.5">
      <ChartFrame
        title="Closure time vs peers"
        headline={model.headline}
        claim="statistical"
        sources={{ tables: ['requests', 'statutory_extensions'], note: 'agency profile aggregate; typical range is the peer mean ± 2 sd' }}
        summary={`${model.headline} ${peers.length} measured peer agencies in this portal; ${unmeasured.length} with no measurable closure are not placed.`}
        status={focus.averageClosureDays === null && peers.length === 0 ? 'empty' : 'ready'}
        emptyText="No agency in this portal has a measurable closure."
        table={{
          caption: 'Average closure days by agency in this portal',
          columns: [{ key: 'agency', label: 'Agency' }, { key: 'days', label: 'Average closure', kind: 'days' }, { key: 'n', label: 'Measured closures', kind: 'count' }, { key: 'z', label: 'z against peers', kind: 'z' }],
          rows,
        }}
        legend={[
          { key: 'this', label: 'This agency', swatch: 'dot', tone: 'ink' },
          { key: 'peers', label: 'Peer agencies', swatch: 'hollow', tone: 'context' },
          ...(model.outlier ? [{ key: 'z', label: 'Slower than peers (statistical)', swatch: 'ring' as const, tone: 'ink' as const }] : []),
        ]}
        minHeight={HEIGHT}
        interactive={peers.length > 0 ? { roledescription: 'dot plot' } : undefined}
        storageKey="inspector-agency-peers"
      >
        {({ width }) => <StripPlot model={model} width={width} onOpenPeer={onOpenPeer} />}
      </ChartFrame>
      {focus.averageClosureDays === null && <p className="text-caption text-muted">This agency: no measurable closure, so it is not placed on the axis.</p>}
      {unmeasured.length > 0 && <p className="text-caption text-muted">No measurable closure: {unmeasured.length} {unmeasured.length === 1 ? 'agency' : 'agencies'} ({unmeasured.map((profile) => profile.agencyName).join(', ')}), not placed.</p>}
    </div>
  );
}
