import type { ReactNode } from 'react';
import { grantsAccess } from '../../utils/licenseState';
import LicenseGate from './LicenseGate';
import { useLicense } from './LicenseProvider';

/** The workspace mounts only when `grantsAccess` says so; every other state, including loading, is the gate. */
export default function LicenseSwitch({ children }: { children: ReactNode }) {
  const controller = useLicense();
  if (!controller) return null;
  return grantsAccess(controller.status) ? <>{children}</> : <LicenseGate controller={controller} />;
}
