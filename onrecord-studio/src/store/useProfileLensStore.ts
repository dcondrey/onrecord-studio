import { create } from 'zustand';

interface ProfileLensState {
  lensEnabled: boolean;
  settingsOpen: boolean;
  selfReviewOpen: boolean;
  /** Bumped after a profile mutation so other subscribers refetch the disclosure. */
  profileRevision: number;
  setLensEnabled: (enabled: boolean) => void;
  setSettingsOpen: (open: boolean) => void;
  setSelfReviewOpen: (open: boolean) => void;
  bumpProfileRevision: () => void;
}

export const useProfileLensStore = create<ProfileLensState>((set) => ({
  lensEnabled: false,
  settingsOpen: false,
  selfReviewOpen: false,
  profileRevision: 0,
  setLensEnabled: (lensEnabled) => set({ lensEnabled }),
  setSettingsOpen: (settingsOpen) => set({ settingsOpen }),
  setSelfReviewOpen: (selfReviewOpen) => set({ selfReviewOpen }),
  bumpProfileRevision: () => set((state) => ({ profileRevision: state.profileRevision + 1 })),
}));
