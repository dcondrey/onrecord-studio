export const OPEN_HELP_EVENT = 'onrecord:open-help';

export interface OpenHelpDetail {
  topicId?: string;
}

/** Any control can open Help without importing it; HelpCenter listens for this one event. */
export function openHelp(topicId?: string) {
  window.dispatchEvent(new CustomEvent<OpenHelpDetail>(OPEN_HELP_EVENT, { detail: { topicId } }));
}
