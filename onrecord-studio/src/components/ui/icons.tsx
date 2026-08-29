import type { SVGProps } from 'react';

export type IconName =
  | 'activity'
  | 'arrow'
  | 'chevron'
  | 'collapse'
  | 'clipboard'
  | 'database'
  | 'evidence'
  | 'eject'
  | 'filePlus'
  | 'globe'
  | 'grid'
  | 'menu'
  | 'moon'
  | 'pause'
  | 'play'
  | 'search'
  | 'shield'
  | 'sliders'
  | 'sparkles'
  | 'sync'
  | 'sun'
  | 'system'
  | 'trash'
  | 'volume'
  | 'x';

interface IconProps extends SVGProps<SVGSVGElement> {
  name: IconName;
}

export default function Icon({ name, className = 'h-4 w-4', ...props }: IconProps) {
  const paths: Record<IconName, JSX.Element> = {
    activity: <><path d="M3 12h3l2.2-6 4.1 12 2.2-6H21" /><path d="M4 4v16h16" opacity=".25" /></>,
    arrow: <><path d="M5 12h14" /><path d="m14 7 5 5-5 5" /></>,
    chevron: <path d="m9 18 6-6-6-6" />,
    clipboard: <><rect x="6" y="4" width="12" height="17" rx="2" /><path d="M9 4.5V3h6v1.5M9 9h6M9 13h6M9 17h4" /></>,
    collapse: <><path d="M9 18 3 12l6-6" /><path d="M21 6v12" /></>,
    database: <><ellipse cx="12" cy="5" rx="8" ry="3" /><path d="M4 5v6c0 1.7 3.6 3 8 3s8-1.3 8-3V5" /><path d="M4 11v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6" /></>,
    evidence: <><path d="M7 3h8l4 4v14H7z" /><path d="M15 3v5h5" /><path d="M10 12h6M10 16h6" /></>,
    eject: <><path d="M9 21H6a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3" /><path d="m15 16 5-4-5-4" /><path d="M20 12H10" /></>,
    filePlus: <><path d="M6 3h8l4 4v14H6z" /><path d="M14 3v5h5M9 14h6M12 11v6" /></>,
    globe: <><circle cx="12" cy="12" r="9" /><path d="M3 12h18M12 3a15 15 0 0 1 0 18M12 3a15 15 0 0 0 0 18" /></>,
    grid: <><rect x="3" y="3" width="7" height="7" rx="1" /><rect x="14" y="3" width="7" height="7" rx="1" /><rect x="3" y="14" width="7" height="7" rx="1" /><rect x="14" y="14" width="7" height="7" rx="1" /></>,
    menu: <><path d="M4 7h16M4 12h16M4 17h16" /></>,
    moon: <path d="M20.2 15.2A8.5 8.5 0 0 1 8.8 3.8 8.5 8.5 0 1 0 20.2 15.2Z" />,
    pause: <><path d="M9 5v14M15 5v14" /></>,
    play: <path d="M8 5.5v13l10-6.5z" />,
    search: <><circle cx="11" cy="11" r="7" /><path d="m20 20-4-4" /></>,
    shield: <><path d="M12 3 5 6v5c0 4.6 2.9 8.2 7 10 4.1-1.8 7-5.4 7-10V6z" /><path d="m9 12 2 2 4-4" /></>,
    sliders: <><path d="M4 7h4M12 7h8M4 12h10M18 12h2M4 17h6M14 17h6" /><circle cx="10" cy="7" r="2" /><circle cx="16" cy="12" r="2" /><circle cx="12" cy="17" r="2" /></>,
    sparkles: <><path d="m12 3 1.1 3.1L16 7.5l-2.9 1.4L12 12l-1.1-3.1L8 7.5l2.9-1.4z" /><path d="m18.5 13 .7 2 1.8.8-1.8.9-.7 2-.7-2-1.8-.9 1.8-.8zM5.5 14l.8 2.2 2.2.8-2.2.8L5.5 20l-.8-2.2-2.2-.8 2.2-.8z" /></>,
    sync: <><path d="M20 7h-5V2" /><path d="M19 7a8 8 0 1 0 1 8" /><path d="M4 17h5v5" /></>,
    sun: <><circle cx="12" cy="12" r="3.5" /><path d="M12 2v2M12 20v2M4.93 4.93l1.42 1.42M17.65 17.65l1.42 1.42M2 12h2M20 12h2M4.93 19.07l1.42-1.42M17.65 6.35l1.42-1.42" /></>,
    system: <><rect x="3" y="4" width="18" height="13" rx="2" /><path d="M8 21h8M12 17v4" /></>,
    trash: <><path d="M4 7h16" /><path d="M9 7V4h6v3" /><path d="M6 7v13a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1V7" /><path d="M10 11v6M14 11v6" /></>,
    volume: <><path d="M5 10v4h3l4 4V6l-4 4z" /><path d="M16 9a4 4 0 0 1 0 6M18.5 6.5a8 8 0 0 1 0 11" /></>,
    x: <path d="m6 6 12 12M18 6 6 18" />,
  };

  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
      {...props}
    >
      {paths[name]}
    </svg>
  );
}
