import findings from '../../content/help/screenshots/findings.jpg';
import tracker from '../../content/help/screenshots/tracker.jpg';
import briefing from '../../content/help/screenshots/briefing.jpg';

export interface HelpShot {
  src: string;
  alt: string;
  caption: string;
}

const SAMPLE = 'Sample data for illustration; not from a real source.';

/** Screenshots bundled with the app, keyed by the `screenshot` field of a topic. */
export const HELP_SHOTS: Record<string, HelpShot> = {
  findings: {
    src: findings,
    alt: 'The Findings inbox with three sample findings. The first carries a red Legal claim: counted violation chip and a 90 percent measured confidence meter. The second is a Pattern signal, not a legal claim with 78 percent measured. The third is a Pattern signal whose confidence reads Unassessed.',
    caption: `The Findings inbox. ${SAMPLE}`,
  },
  tracker: {
    src: tracker,
    alt: 'The My requests dialog showing one tracked request with a chip reading 5 days left (provisional), a timeline with one submitted date, and a suggested event from an email marked Suggested, not counted until you confirm.',
    caption: `A tracked request with a suggestion waiting for confirmation. ${SAMPLE}`,
  },
  briefing: {
    src: briefing,
    alt: 'The top of the Briefing view: the heading Turn evidence into a defensible public comment, buttons to use a template and export an outline, and the Frame the presentation form.',
    caption: 'The Briefing view, top of the page.',
  },
};
