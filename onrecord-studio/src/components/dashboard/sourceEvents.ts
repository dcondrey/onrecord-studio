/** Any surface can ask the header to put the cursor in the source field, at every window width. */
export const OPEN_SOURCE_CONNECTOR_EVENT = 'onrecord:open-source-connector';

export function openSourceConnector() {
  window.dispatchEvent(new Event(OPEN_SOURCE_CONNECTOR_EVENT));
}
