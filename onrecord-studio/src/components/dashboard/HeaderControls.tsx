import Button, { buttonClass } from '../ui/Button';
import { cn } from '../../utils/cn';
import CaseSwitcher from './CaseSwitcher';
import Icon from '../ui/icons';
import { useProfileLensStore } from '../../store/useProfileLensStore';
import ProfileSettings from '../account/ProfileSettings';
import SelfReview from '../account/SelfReview';
import AccountMenu from '../account/AccountMenu';
import MyRequestsButton from '../tracker/MyRequestsButton';
import HelpButton from '../help/HelpButton';
import UpdatePrompt from '../updates/UpdatePrompt';

/** The right-hand icon row. Every trigger is a 32px ghost button, so the row reads as one
 * toolbar; audio alerts live in Account and appearance in the View menu and the command palette.
 * Below 2xl the source form folds behind "Add source". */
export function HeaderControls({
  syncingPortalKeys,
  isConnectorOpen,
  onToggleConnector,
}: {
  syncingPortalKeys: string[];
  isConnectorOpen: boolean;
  onToggleConnector: () => void;
}) {
  const setProfileOpen = useProfileLensStore((state) => state.setSettingsOpen);
  return (
    <div className="ml-auto flex shrink-0 items-center gap-1">
      <button
        type="button"
        aria-expanded={isConnectorOpen}
        onClick={onToggleConnector}
        className={buttonClass({ variant: 'secondary', className: cn('focus-ring mr-2 2xl:hidden', isConnectorOpen && 'bg-raised') })}
      >
        <Icon name="filePlus" size="sm" />
        <span>{syncingPortalKeys.length > 0 ? `Sources · ${syncingPortalKeys.length} syncing` : 'Add source'}</span>
      </button>
      <CaseSwitcher />
      <Button
        variant="ghost"
        iconOnly
        icon="shield"
        title="Local profile and declared organization"
        aria-label="Open local profile"
        onClick={() => setProfileOpen(true)}
        className="focus-ring"
      />
      <HelpButton />
      <MyRequestsButton />
      <AccountMenu />
      <UpdatePrompt />
      <ProfileSettings />
      <SelfReview />
    </div>
  );
}
