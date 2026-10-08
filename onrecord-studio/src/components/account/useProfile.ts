import { useCallback, useEffect, useState } from "react";
import { formatNativeError, invokeCommand } from "../../ipc";
import { useLatestRequestGate } from "../../hooks/useLatestRequestGate";
import { useProfileLensStore } from "../../store/useProfileLensStore";
import type {
  AffiliationDisclosure,
  AffiliationMatchDecisionInput,
  AffiliationMatchList,
  Finding,
  ProfileInput,
  ProfileState,
} from "../../types";

/**
 * Whether a finding's subject is the user's own confirmed organization. A confirmed match
 * resolves to a portal, an agency within a portal, or an entity.
 */
export function touchesOwnOrganization(
  finding: Finding,
  disclosure: AffiliationDisclosure | null,
): boolean {
  if (!disclosure) return false;
  const { portalKey, agencyId, entityId } = finding.subject;
  return (
    (portalKey !== null && disclosure.ownPortalKeys.includes(portalKey)) ||
    (portalKey !== null &&
      agencyId !== null &&
      disclosure.ownAgencies.some(
        (agency) =>
          agency.portalKey === portalKey && agency.agencyId === agencyId,
      )) ||
    (entityId !== null && disclosure.ownEntityIds.includes(entityId))
  );
}

function isDisclosure(value: unknown): value is AffiliationDisclosure {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Partial<AffiliationDisclosure>;
  return (
    Array.isArray(candidate.ownPortalKeys) &&
    Array.isArray(candidate.ownAgencies) &&
    Array.isArray(candidate.ownEntityIds) &&
    Array.isArray(candidate.affiliations)
  );
}

export type DisclosureStatus = "loading" | "loaded" | "failed";

/**
 * Read-only disclosure for lens consumers. Refetches when the profile changes. `disclosure` is
 * `null` both while loading and for no profile, so a consumer that wants to say "no profile"
 * must check `status === "loaded"` first.
 */
export function useAffiliationDisclosure() {
  const gate = useLatestRequestGate();
  const revision = useProfileLensStore((state) => state.profileRevision);
  const [disclosure, setDisclosure] = useState<AffiliationDisclosure | null>(
    null,
  );
  const [status, setStatus] = useState<DisclosureStatus>("loading");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const token = gate.begin();
    setStatus("loading");
    invokeCommand("get_affiliation_disclosure", {})
      .then((value) => {
        if (!gate.isCurrent(token)) return;
        // A malformed answer is "no profile", never a guessed affiliation.
        setDisclosure(isDisclosure(value) ? value : null);
        setError(null);
        setStatus("loaded");
      })
      .catch((reason) => {
        if (!gate.isCurrent(token)) return;
        setError(formatNativeError(reason));
        setStatus("failed");
      });
  }, [gate, revision]);

  return { disclosure, status, error };
}

export function useProfile() {
  const gate = useLatestRequestGate();
  const bumpProfileRevision = useProfileLensStore(
    (state) => state.bumpProfileRevision,
  );
  const [state, setState] = useState<ProfileState | null>(null);
  const [matches, setMatches] = useState<AffiliationMatchList | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isBusy, setIsBusy] = useState(false);

  const refresh = useCallback(() => {
    const token = gate.begin();
    setError(null);
    Promise.all([
      invokeCommand("get_profile", {}),
      invokeCommand("list_affiliation_matches", {}),
    ])
      .then(([profileState, matchList]) => {
        if (!gate.isCurrent(token)) return;
        setState(profileState);
        setMatches(matchList);
      })
      .catch((reason) => {
        if (!gate.isCurrent(token)) return;
        setError(formatNativeError(reason));
      });
  }, [gate]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const mutate = useCallback(
    async (operation: () => Promise<unknown>) => {
      setIsBusy(true);
      setError(null);
      try {
        await operation();
        bumpProfileRevision();
        refresh();
        return true;
      } catch (reason) {
        setError(formatNativeError(reason));
        return false;
      } finally {
        setIsBusy(false);
      }
    },
    [bumpProfileRevision, refresh],
  );

  return {
    state,
    /** False until the stored profile has been read; an unread profile is not an empty one. */
    loaded: state !== null,
    matches,
    error,
    isBusy,
    refresh,
    save: (input: ProfileInput) =>
      mutate(() => invokeCommand("save_profile", { input })),
    remove: () => mutate(() => invokeCommand("delete_profile", {})),
    confirm: (input: AffiliationMatchDecisionInput) =>
      mutate(() => invokeCommand("confirm_affiliation_match", { input })),
    reject: (input: AffiliationMatchDecisionInput) =>
      mutate(() => invokeCommand("reject_affiliation_match", { input })),
  };
}
