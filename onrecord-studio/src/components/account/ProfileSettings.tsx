import { useCallback, useEffect, useId, useRef, useState } from 'react';
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
import Button from '../ui/Button';
import Dialog, { DialogBody, DialogFooter, DialogHeader } from '../ui/Dialog';
import { Input } from '../ui/Field';
import InlineError from '../ui/InlineError';
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
    <li className="rounded-lg border border-line p-3 text-body">
      <div className="font-medium text-fg">{candidate.displayValue}</div>
      <div className="mt-0.5 text-caption text-muted">
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
      <div className="mt-2 flex items-center gap-2">
        <span className="mr-auto text-caption text-fg-secondary">
          {candidate.decision === 'PENDING'
            ? 'Undecided'
            : candidate.decision === 'CONFIRMED'
              ? 'Confirmed'
              : 'Rejected'}
        </span>
        <Button size="sm" disabled={disabled || candidate.decision === 'CONFIRMED'} onClick={onConfirm}>
          Confirm
        </Button>
        <Button size="sm" disabled={disabled || candidate.decision === 'REJECTED'} onClick={onReject}>
          Reject
        </Button>
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

function sameDraft(left: ReturnType<typeof draftFromProfile>, right: ReturnType<typeof draftFromProfile>): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

/** A field's label with its character count on the same line; the count describes the field, it does not name it. */
function LabelRow({ htmlFor, label, countId, used, limit }: { htmlFor?: string; label: string; countId?: string; used: number; limit: number }) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      {htmlFor ? <label htmlFor={htmlFor} className="text-control text-fg-secondary">{label}</label> : <span className="text-control text-fg-secondary">{label}</span>}
      <span id={countId} className="text-caption tabular-nums text-muted">{used}/{limit}</span>
    </div>
  );
}

export function ProfileSettingsDialog({ onClose }: { onClose: () => void }) {
  const profile = useProfile();
  const setSelfReviewOpen = useProfileLensStore((state) => state.setSelfReviewOpen);
  const [draft, setDraft] = useState(() => draftFromProfile(null));
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [saved, setSaved] = useState(false);
  const savedTimer = useRef<number | null>(null);
  const ids = useId();
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

  useEffect(() => () => {
    if (savedTimer.current !== null) window.clearTimeout(savedTimer.current);
  }, []);

  const updateAffiliation = (index: number, next: AffiliationInput) =>
    setDraft((current) => ({
      ...current,
      affiliations: current.affiliations.map((a, i) => (i === index ? next : a)),
    }));

  // Saving before the stored profile is read would write an empty profile over it.
  const dirty = profile.loaded && !sameDraft(draft, draftFromProfile(stored));
  const failedToRead = !profile.loaded && Boolean(profile.error);
  const save = async () => {
    const ok = await profile.save({
      displayName: draft.displayName.trim() === '' ? null : draft.displayName,
      affiliations: draft.affiliations,
    });
    if (!ok) return;
    setSaved(true);
    if (savedTimer.current !== null) window.clearTimeout(savedTimer.current);
    savedTimer.current = window.setTimeout(() => setSaved(false), 2000);
  };

  return (
    <Dialog onClose={onClose} size="lg">
      <DialogHeader
        title="Profile"
        description="Affiliation is self-declared and stored only on this Mac. It never changes a finding, confidence, severity, statutory result or ranking; it only filters, highlights and adds a disclosure to what you export."
      />
      <DialogBody className="text-body text-fg-secondary">
        {failedToRead ? (
          <InlineError summary={`Your profile could not be read. ${profile.error}`} onRetry={profile.refresh} />
        ) : !profile.loaded ? (
          <div role="status" aria-busy="true" className="flex flex-col gap-3">
            <span className="sr-only">Loading your profile</span>
            <div className="skeleton h-9 rounded-lg" aria-hidden="true" />
            <div className="skeleton h-9 rounded-lg" aria-hidden="true" />
          </div>
        ) : (
          <>
            {profile.error && <p role="alert" className="mb-3 text-danger">{profile.error}</p>}
            {profile.state?.status === 'UNSET' && <p className="mb-4 text-muted">No profile is set yet. Add a name or an organization and save to create one.</p>}
            <LabelRow htmlFor={`${ids}-name`} label="Display name" countId={`${ids}-name-count`} used={draft.displayName.length} limit={PROFILE_LIMITS.displayName} />
            <Input
              id={`${ids}-name`}
              value={draft.displayName}
              maxLength={PROFILE_LIMITS.displayName}
              aria-describedby={`${ids}-name-count`}
              onChange={(event) => setDraft({ ...draft, displayName: event.target.value })}
              className="mt-1.5"
            />

            <div className="mt-6 flex items-baseline justify-between">
              <h3 className="text-heading text-fg">Organizations</h3>
              <span className="text-caption tabular-nums text-muted">{draft.affiliations.length}/{PROFILE_LIMITS.affiliations}</span>
            </div>
            {draft.affiliations.map((affiliation, index) => (
              <fieldset key={affiliation.id ?? `new-${index}`} className="mt-3 rounded-xl border border-line bg-panel/60 p-4">
                <LabelRow htmlFor={`${ids}-org-${index}`} label="Organization name" countId={`${ids}-org-${index}-count`} used={affiliation.name.length} limit={PROFILE_LIMITS.affiliationName} />
                <Input
                  id={`${ids}-org-${index}`}
                  value={affiliation.name}
                  maxLength={PROFILE_LIMITS.affiliationName}
                  aria-describedby={`${ids}-org-${index}-count`}
                  onChange={(event) => updateAffiliation(index, { ...affiliation, name: event.target.value })}
                  className="mt-1.5"
                />
                <div className="mt-4">
                  <LabelRow label="Aliases" used={affiliation.aliases.length} limit={PROFILE_LIMITS.aliasesPerAffiliation} />
                </div>
                {affiliation.aliases.map((alias, aliasIndex) => (
                  <div key={aliasIndex} className="mt-1.5 flex items-center gap-1">
                    <Input
                      aria-label={`Alias ${aliasIndex + 1} of ${affiliation.name || 'organization'}`}
                      value={alias}
                      maxLength={PROFILE_LIMITS.alias}
                      onChange={(event) =>
                        updateAffiliation(index, {
                          ...affiliation,
                          aliases: affiliation.aliases.map((a, i) => (i === aliasIndex ? event.target.value : a)),
                        })
                      }
                    />
                    <Button
                      variant="ghost"
                      iconOnly
                      icon="x"
                      aria-label={`Remove alias ${aliasIndex + 1}`}
                      onClick={() =>
                        updateAffiliation(index, {
                          ...affiliation,
                          aliases: affiliation.aliases.filter((_, i) => i !== aliasIndex),
                        })
                      }
                    />
                  </div>
                ))}
                <div className="mt-2 flex items-center gap-2">
                  <Button
                    size="sm"
                    disabled={affiliation.aliases.length >= PROFILE_LIMITS.aliasesPerAffiliation}
                    onClick={() => updateAffiliation(index, { ...affiliation, aliases: [...affiliation.aliases, ''] })}
                  >
                    Add alias
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="ml-auto text-danger hover:bg-danger/10 hover:text-danger"
                    onClick={() =>
                      setDraft((current) => ({
                        ...current,
                        affiliations: current.affiliations.filter((_, i) => i !== index),
                      }))
                    }
                  >
                    Remove organization
                  </Button>
                </div>
                <WorkEmailVerification
                  affiliation={stored?.affiliations.find((a) => a.id === affiliation.id) ?? null}
                  verifiedDomains={stored?.verifiedDomains ?? []}
                  onProfileChanged={onVerificationChanged}
                />
              </fieldset>
            ))}
            <Button
              size="sm"
              disabled={draft.affiliations.length >= PROFILE_LIMITS.affiliations}
              onClick={() =>
                setDraft((current) => ({
                  ...current,
                  affiliations: [...current.affiliations, { id: null, name: '', aliases: [] }],
                }))
              }
              className="mt-2"
            >
              Add organization
            </Button>

            {/* Matching needs a declared name to compare; before a profile exists there is nothing to report. */}
            {stored && <h3 className="mt-6 text-heading text-fg">Matches</h3>}
            {stored && profile.matches && (
              <div className="mt-1">
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
                  <ul className="mt-2 list-disc pl-4 text-caption text-muted" aria-label="Match limitations">
                    {profile.matches.limitations.map((line) => (
                      <li key={line}>{line}</li>
                    ))}
                  </ul>
                )}
              </div>
            )}

            <div className="mt-5 flex flex-wrap items-center gap-2 border-t border-line pt-4">
              <Button
                size="sm"
                onClick={() => {
                  setSelfReviewOpen(true);
                  onClose();
                }}
              >
                Open records self-review
              </Button>
              {!stored ? null : !confirmingDelete ? (
                <Button variant="danger" size="sm" onClick={() => setConfirmingDelete(true)} className="ml-auto">
                  Delete local profile
                </Button>
              ) : (
                <div className="ml-auto flex flex-wrap items-center gap-2" role="group" aria-label="Confirm profile deletion">
                  <span className="text-caption">Delete the profile file and every confirmed or rejected match on this device?</span>
                  <Button
                    variant="destructive"
                    size="sm"
                    onClick={() => {
                      setConfirmingDelete(false);
                      void profile.remove();
                    }}
                  >
                    Yes, delete
                  </Button>
                  <Button size="sm" onClick={() => setConfirmingDelete(false)}>
                    Keep
                  </Button>
                </div>
              )}
            </div>
          </>
        )}
      </DialogBody>
      <DialogFooter>
        {failedToRead ? (
          <Button onClick={onClose}>Close</Button>
        ) : (
          <>
            {saved && <p role="status" className="mr-auto text-caption text-success">Saved</p>}
            <Button onClick={onClose}>Cancel</Button>
            <Button variant="primary" disabled={!dirty || profile.isBusy} onClick={() => void save()}>
              Save
            </Button>
          </>
        )}
      </DialogFooter>
    </Dialog>
  );
}
