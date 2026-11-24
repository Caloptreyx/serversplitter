import type { AddToast } from '@/providers/contexts/toastContext.ts';
import Select from '@/elements/input/Select.tsx';
import type { MasterPower, MasterPowerAction, MasterPowerOptions, MasterPowerResult } from '../api/client.ts';

export const MASTER_POWER_LABELS: Record<MasterPower, string> = {
  none: 'Leave the master as it is',
  restart: 'Restart the master',
  stop: 'Stop the master',
  kill: 'Kill the master',
};

const DOING: Record<MasterPowerAction, string> = { restart: 'restarting', stop: 'stopping', kill: 'killing' };
const DONE: Record<MasterPowerAction, string> = { restart: 'restarting', stop: 'stopping', kill: 'killed' };

/** The choice is offered only when an action is allowed and the master is up. */
export function masterPowerOffered(options: MasterPowerOptions | null, state: string | null): boolean {
  return !!options && options.allowed.length > 0 && (state === 'running' || state === 'starting');
}

/** The admin's default when it's allowed, else `none`. */
export function initialMasterPower(options: MasterPowerOptions | null): MasterPower {
  const preset = options?.default ?? 'none';
  return preset !== 'none' && options?.allowed.includes(preset) ? preset : 'none';
}

/**
 * Toasts a split change that succeeded, with the outcome of the power action sent after it.
 * A failed power action is a warning: the split change itself went through.
 */
export function reportMasterPower(addToast: AddToast, message: string, result: MasterPowerResult | null) {
  if (!result) {
    addToast(message, 'success');
  } else if (result.ok) {
    addToast(`${message}, master ${DONE[result.action]}`, 'success');
  } else {
    addToast(
      `${message}, but ${DOING[result.action]} the master failed${result.error ? `: ${result.error}` : ''}`,
      'warning',
    );
  }
}

export default function MasterPowerSelect({
  options,
  value,
  onChange,
}: {
  options: MasterPowerOptions;
  value: MasterPower;
  onChange: (value: MasterPower) => void;
}) {
  return (
    <Select
      label='Then'
      description='Sent to the master once the split change is saved.'
      data={(['none', ...options.allowed] as MasterPower[]).map((power) => ({
        value: power,
        label: MASTER_POWER_LABELS[power],
      }))}
      value={value}
      onChange={(next) => next && onChange(next as MasterPower)}
    />
  );
}
