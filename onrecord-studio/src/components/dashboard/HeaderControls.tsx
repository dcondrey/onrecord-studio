import CaseSwitcher from './CaseSwitcher';
import Button from '../ui/Button';
import ProfileSettings from '../account/ProfileSettings';
import SelfReview from '../account/SelfReview';
import AccountMenu from '../account/AccountMenu';
import MyRequestsButton from '../tracker/MyRequestsButton';
import HelpButton from '../help/HelpButton';
import UpdatePrompt from '../updates/UpdatePrompt';

/**
 * The right-hand toolbar: case, requests, help, account, then the one primary action. Profile is
 * reached through Account, audio alerts and appearance through the command palette, and source
 * status lives on each source row in the sidebar.
 */
export function HeaderControls({ isConnectorOpen, onOpenConnector }: { isConnectorOpen: boolean; onOpenConnector: () => void }) {
  return (
    <div className="ml-auto flex min-w-0 items-center gap-2">
      <CaseSwitcher />
      <MyRequestsButton />
      <HelpButton />
      <AccountMenu />
      <UpdatePrompt />
      <ProfileSettings />
      <SelfReview />
      <Button
        variant="primary"
        icon="filePlus"
        aria-haspopup="dialog"
        aria-expanded={isConnectorOpen}
        aria-keyshortcuts="Meta+N"
        onClick={onOpenConnector}
        className="whitespace-nowrap"
      >
        Add source
      </Button>
    </div>
  );
}
