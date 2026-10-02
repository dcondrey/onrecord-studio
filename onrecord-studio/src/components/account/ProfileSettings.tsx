import { useCallback, useEffect, useState } from 'react';
import { useProfileLensStore } from '../../store/useProfileLensStore';
import {
  PROFILE_LIMITS,
  type AffiliationInput,
  type AffiliationMatchCandidate,
  type AffiliationMatchList,
  type AffiliationMatchMethod,
  type EntityValidationGrade,
  type UserProfile,
} from '../../types';
import { useDialogFocus } from '../licensing/useDialogFocus';
import { useProfile } from './useProfile';
import WorkEmailVerification from './WorkEmailVerification';

const methodLabels: Record<AffiliationMatchMethod, string> = {
  EXACT: 'Exact name match',
  ALIAS: 'Alias match',
  FUZZY: 'Similar name (fuzzy)',
  PREVIOUSLY_DECIDED: 'Not in current sources; decided earlier',
};

const gradeLabels: Record<EntityValidationGrade, string> = {
  CHECK_DIGIT_PASSED: 'check digit passed',
  FORMAT_VALIDATED: 'format validated',
  PATTERN_ONLY: 'pattern only (a lead)',
  GAZETTEER_HIT: 'word-list match, not validated',
};

function draftFromProfile(profile: UserProfile | null): {
  displayName: string;
  affiliations: AffiliationInput[];
} {
  return {
    displayName: profile?.displayName ?? '',
    affiliations:
      profile?.affiliations.map((a) => ({ id: a.id, name: a.name, aliases: [...a.aliases] })) ?? [],
  };
}

export function MatchStatusNotice({ list }: { list: AffiliationMatchList }) {
  switch (list.status) {
    case 'UNSET':
      return <p role="status">No profile is set, so nothing was matched.</p>;
    case 'NO_AFFILIATIONS_DECLARED':
      return <p role="status">The profile declares no organization, so nothing was matched.</p>;
    case 'NO_MATCH_YET':
      return (
        <p role="status">
          Sources were searched ({list.searchedPortalCount} registered sources, {list.searchedAgencyCount} agencies,{' '}
          {list.searchedEntityCount} extracted entities) and nothing resembled a declared name. This
          is not a statement about what any record contains.
        </p>
      );
    case 'HAS_CANDIDATES':
      return (
        <p role="status">
          {list.candidates.length} candidate{list.candidates.length === 1 ? '' : 's'} found. None
          applies until you confirm it.
        </p>
      );
  }
}

function CandidateRow({
  candidate,
  disabled,
  onConfirm,
  onReject,
}: {
  candidate: AffiliationMatchCandidate;
  disabled: boolean;
  onConfirm: () => void;
  onReject: () => void;
}) {
  return (
    <li className="rounded-lg border border-slate-800 p-2 text-label">
      <div className="font-semibold text-slate-100">{candidate.displayValue}</div>
      <div className="text-slate-400">
        {candidate.kind === 'PORTAL'
          ? 'Registered source'
          : candidate.kind === 'AGENCY'
            ? 'Agency on a source'
            : 'Organization in documents'} ·{' '}
        {methodLabels[candidate.method]} (compared with &quot;{candidate.matchedTerm}&quot;) ·{' '}
        {candidate.validationGrade
          ? gradeLabels[candidate.validationGrade]
          : 'registered source, not extracted'}
        {candidate.documentCount !== null && ` · ${candidate.documentCount} documents`}
      </div>
      <div className="mt-1 flex items-center gap-2">
        <span className="text-slate-300">
          {candidate.decision === 'PENDING'
            ? 'Undecided'
            : candidate.decision === 'CONFIRMED'
              ? 'Confirmed'
              : 'Rejected'}
        </span>
        <button
          type="button"
          disabled={disabled || candidate.decision === 'CONFIRMED'}
          onClick={onConfirm}
          className="focus-ring rounded border border-slate-700 px-2 py-0.5"
        >
          Confirm
        </button>
        <button
          type="button"
          disabled={disabled || candidate.decision === 'REJECTED'}
          onClick={onReject}
          className="focus-ring rounded border border-slate-700 px-2 py-0.5"
        >
          Reject
        </button>
      </div>
    </li>
  );
}

export default function ProfileSettings() {
  const open = useProfileLensStore((state) => state.settingsOpen);
  const setOpen = useProfileLensStore((state) => state.setSettingsOpen);
  if (!open) return null;
  return <ProfileSettingsDialog onClose={() => setOpen(false)} />;
}

export function ProfileSettingsDialog({ onClose }: { onClose: () => void }) {
  const profile = useProfile();
  const dialogFocus = useDialogFocus(onClose);
  const setSelfReviewOpen = useProfileLensStore((state) => state.setSelfReviewOpen);
  const [draft, setDraft] = useState(() => draftFromProfile(null));
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const stored = profile.state?.profile ?? null;
  const bumpProfileRevision = useProfileLensStore((state) => state.bumpProfileRevision);
  const refreshProfile = profile.refresh;
  const onVerificationChanged = useCallback(() => {
    bumpProfileRevision();
    refreshProfile();
  }, [bumpProfileRevision, refreshProfile]);

  useEffect(() => {
    if (profile.state) setDraft(draftFromProfile(profile.state.profile));
  }, [profile.state]);

  const updateAffiliation = (index: number, next: AffiliationInput) =>
    setDraft((current) => ({
      ...current,
      affiliations: current.affiliations.map((a, i) => (i === index ? next : a)),
    }));

  return (
    <div role="dialog" aria-modal="true" aria-label="Local profile" className="fixed inset-0 z-50 grid place-items-center bg-black/60 p-4" onKeyDown={dialogFocus.onKeyDown}>
      <div ref={dialogFocus.ref} tabIndex={-1} className="app-panel max-h-[90vh] w-full max-w-2xl overflow-y-auto p-4 text-label text-slate-300 outline-none">
        <div className="flex items-center justify-between">
          <h2 className="text-label font-semibold text-slate-100">Local profile</h2>
          <button type="button" onClick={onClose} className="focus-ring rounded border border-slate-700 px-2 py-1">
            Close
          </button>
        </div>
        <p className="mt-2 text-label leading-relaxed text-slate-400">
          Affiliation is self-declared: you type the name and nothing checks it against a list. It
          is stored only on this device and never transmitted. It never changes any finding,
          confidence, severity, statutory result or ranking. It only filters, highlights and adds a
          disclosure to what you export.
        </p>
        {profile.error && (
          <div role="alert" className="mt-2 text-rose-300">
            {profile.error}
          </div>
        )}
        {profile.state?.status === 'UNSET' && <p className="mt-2">No profile is set.</p>}

        <label className="mt-3 block">
          Display name ({draft.displayName.length}/{PROFILE_LIMITS.displayName})
          <input
            value={draft.displayName}
            maxLength={PROFILE_LIMITS.displayName}
            onChange={(event) => setDraft({ ...draft, displayName: event.target.value })}
            className="mt-1 w-full rounded border border-slate-700 bg-slate-950 px-2 py-1"
          />
        </label>

        <h3 className="mt-4 font-semibold text-slate-200">
          Organizations ({draft.affiliations.length}/{PROFILE_LIMITS.affiliations})
        </h3>
        {draft.affiliations.map((affiliation, index) => (
          <fieldset key={affiliation.id ?? `new-${index}`} className="mt-2 rounded-lg border border-slate-800 p-2">
            <label className="block">
              Organization name ({affiliation.name.length}/{PROFILE_LIMITS.affiliationName})
              <input
                value={affiliation.name}
                maxLength={PROFILE_LIMITS.affiliationName}
                onChange={(event) => updateAffiliation(index, { ...affiliation, name: event.target.value })}
                className="mt-1 w-full rounded border border-slate-700 bg-slate-950 px-2 py-1"
              />
            </label>
            <div className="mt-1 text-slate-400">
              Aliases ({affiliation.aliases.length}/{PROFILE_LIMITS.aliasesPerAffiliation})
            </div>
            {affiliation.aliases.map((alias, aliasIndex) => (
              <div key={aliasIndex} className="mt-1 flex gap-1">
                <input
                  aria-label={`Alias ${aliasIndex + 1} of ${affiliation.name || 'organization'}`}
                  value={alias}
                  maxLength={PROFILE_LIMITS.alias}
                  onChange={(event) =>
                    updateAffiliation(index, {
                      ...affiliation,
                      aliases: affiliation.aliases.map((a, i) => (i === aliasIndex ? event.target.value : a)),
                    })
                  }
                  className="w-full rounded border border-slate-700 bg-slate-950 px-2 py-1"
                />
                <button
                  type="button"
                  onClick={() =>
                    updateAffiliation(index, {
                      ...affiliation,
                      aliases: affiliation.aliases.filter((_, i) => i !== aliasIndex),
                    })
                  }
                  className="focus-ring rounded border border-slate-700 px-2"
                >
                  Remove alias
                </button>
              </div>
            ))}
            <div className="mt-2 flex gap-2">
              <button
                type="button"
                disabled={affiliation.aliases.length >= PROFILE_LIMITS.aliasesPerAffiliation}
                onClick={() => updateAffiliation(index, { ...affiliation, aliases: [...affiliation.aliases, ''] })}
                className="focus-ring rounded border border-slate-700 px-2 py-0.5"
              >
                Add alias
              </button>
              <button
                type="button"
                onClick={() =>
                  setDraft((current) => ({
                    ...current,
                    affiliations: current.affiliations.filter((_, i) => i !== index),
                  }))
                }
                className="focus-ring rounded border border-slate-700 px-2 py-0.5"
              >
                Remove organization
              </button>
            </div>
            <WorkEmailVerification
              affiliation={stored?.affiliations.find((a) => a.id === affiliation.id) ?? null}
              verifiedDomains={stored?.verifiedDomains ?? []}
              onProfileChanged={onVerificationChanged}
            />
          </fieldset>
        ))}
        <div className="mt-2 flex gap-2">
          <button
            type="button"
            disabled={draft.affiliations.length >= PROFILE_LIMITS.affiliations}
            onClick={() =>
              setDraft((current) => ({
                ...current,
                affiliations: [...current.affiliations, { id: null, name: '', aliases: [] }],
              }))
            }
            className="focus-ring rounded border border-slate-700 px-2 py-1"
          >
            Add organization
          </button>
          <button
            type="button"
            disabled={profile.isBusy}
            onClick={() =>
              void profile.save({
                displayName: draft.displayName.trim() === '' ? null : draft.displayName,
                affiliations: draft.affiliations,
              })
            }
            className="focus-ring profile-save rounded border border-cyan-400/40 px-2 py-1 text-cyan-200"
          >
            Save profile
          </button>
        </div>

        <h3 className="mt-4 font-semibold text-slate-200">Matches</h3>
        {profile.matches && (
          <>
            <MatchStatusNotice list={profile.matches} />
            <ul className="mt-2 flex flex-col gap-2" aria-label="Affiliation match candidates">
              {profile.matches.candidates.map((candidate) => {
                const input = {
                  affiliationId: candidate.affiliationId,
                  kind: candidate.kind,
                  normalizedName: candidate.normalizedName,
                };
                return (
                  <CandidateRow
                    key={`${candidate.affiliationId}:${candidate.kind}:${candidate.normalizedName}`}
                    candidate={candidate}
                    disabled={profile.isBusy}
                    onConfirm={() => void profile.confirm(input)}
                    onReject={() => void profile.reject(input)}
                  />
                );
              })}
            </ul>
            {profile.matches.limitations.length > 0 && (
              <ul className="mt-2 list-disc pl-4 text-label text-slate-400" aria-label="Match limitations">
                {profile.matches.limitations.map((line) => (
                  <li key={line}>{line}</li>
                ))}
              </ul>
            )}
          </>
        )}

        <div className="mt-4 flex flex-wrap items-center gap-2 border-t border-slate-800 pt-3">
          <button
            type="button"
            onClick={() => {
              setSelfReviewOpen(true);
              onClose();
            }}
            className="focus-ring rounded border border-slate-700 px-2 py-1"
          >
            Open records self-review
          </button>
          {!confirmingDelete ? (
            <button
              type="button"
              disabled={!stored}
              onClick={() => setConfirmingDelete(true)}
              className="focus-ring ml-auto rounded border border-rose-500/40 px-2 py-1 text-rose-200"
            >
              Delete local profile
            </button>
          ) : (
            <div className="ml-auto flex flex-wrap items-center gap-2" role="alertdialog" aria-label="Confirm profile deletion">
              <span>Delete the profile file and every confirmed or rejected match on this device?</span>
              <button
                type="button"
                onClick={() => {
                  setConfirmingDelete(false);
                  void profile.remove();
                }}
                className="focus-ring rounded border border-rose-500/60 px-2 py-1 text-rose-200"
              >
                Yes, delete
              </button>
              <button type="button" onClick={() => setConfirmingDelete(false)} className="focus-ring rounded border border-slate-700 px-2 py-1">
                Keep
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
