import { faRotate, faTriangleExclamation } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { useEffect, useState } from 'react';
import { httpErrorToHuman } from '@/api/axios.ts';
import Button from '@/elements/buttons/Button.tsx';
import Badge from '@/elements/data-display/Badge.tsx';
import Alert from '@/elements/feedback/Alert.tsx';
import Spinner from '@/elements/feedback/Spinner.tsx';
import Switch from '@/elements/input/Switch.tsx';
import { Modal, ModalFooter } from '@/elements/modals/Modal.tsx';
import { useServerCan } from '@/plugins/usePermissions.ts';
import { useToast } from '@/providers/ToastProvider.tsx';
import {
  getSplitSubusers,
  type SplitEntry,
  type SplitSubuser,
  setSplitSubuserCustom,
  type SyncResult,
  syncSubusers,
} from '../api/client.ts';

/** Toast text for a finished sync, shared with the split row's menu action. */
export function syncResultMessage(result: SyncResult): string {
  const base = `Added ${result.added}, updated ${result.updated}`;
  return result.skipped.length > 0 ? `${base}; skipped ${result.skipped.join(', ')}` : base;
}

export default function SplitSubusersModal({
  split,
  serverUuid,
  masterName,
  onClose,
  onChanged,
}: {
  /** The split whose subusers are shown; `null` keeps the modal closed. */
  split: SplitEntry | null;
  serverUuid: string;
  masterName: string;
  onClose: () => void;
  /** Subusers changed (sync ran), the page reloads its counts. */
  onChanged: () => void;
}) {
  const { addToast } = useToast();
  const canUpdate = useServerCan('splitter.update');
  const canSync = useServerCan(['splitter.update', 'subusers.create', 'subusers.update'], false);

  const [subusers, setSubusers] = useState<SplitSubuser[] | null>(null);
  const [savingUser, setSavingUser] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [skipped, setSkipped] = useState<string[]>([]);

  const splitUuid = split?.server.uuid;

  const load = () => {
    if (!splitUuid) return;
    getSplitSubusers(serverUuid, splitUuid)
      .then(setSubusers)
      .catch((err) => {
        setSubusers([]);
        addToast(httpErrorToHuman(err), 'error');
      });
  };

  useEffect(() => {
    setSubusers(null);
    setSkipped([]);
    load();
  }, [splitUuid]);

  const setCustom = async (user: SplitSubuser, custom: boolean) => {
    if (!splitUuid) return;
    setSavingUser(user.user.uuid);

    try {
      await setSplitSubuserCustom(serverUuid, splitUuid, user.user.uuid, custom);
      setSubusers((prev) => prev?.map((s) => (s.user.uuid === user.user.uuid ? { ...s, custom } : s)) ?? null);
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setSavingUser(null);
    }
  };

  const syncNow = async () => {
    if (!splitUuid) return;
    setSyncing(true);

    try {
      const result = await syncSubusers(serverUuid, splitUuid);
      setSkipped(result.skipped);
      addToast(syncResultMessage(result), result.skipped.length > 0 ? 'warning' : 'success');
      load();
      onChanged();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setSyncing(false);
    }
  };

  return (
    <Modal opened={!!split} onClose={onClose} title={split ? `Subusers of ${split.server.name}` : 'Subusers'} size='lg'>
      <div className='flex flex-col gap-3'>
        <p className='text-xs text-(--mantine-color-dimmed)'>
          Custom: keep this user&apos;s permissions on the split as they are; syncing won&apos;t change or re-add them.
          Removing them from the master still removes them.
        </p>

        {skipped.length > 0 && (
          <Alert color='yellow' icon={faTriangleExclamation} title='Skipped'>
            These {masterName} subusers have permissions you can&apos;t grant: {skipped.join(', ')}
          </Alert>
        )}

        {subusers === null ? (
          <div className='flex justify-center py-6'>
            <Spinner />
          </div>
        ) : subusers.length === 0 ? (
          <p className='py-6 text-center text-sm text-(--mantine-color-dimmed)'>
            Neither {masterName} nor this split has subusers.
          </p>
        ) : (
          <div className='flex flex-col rounded-md border border-(--mantine-color-default-border)'>
            {subusers.map((subuser) => (
              <div
                key={subuser.user.uuid}
                className='flex flex-col sm:flex-row sm:items-center gap-2 px-3 py-2 border-b border-(--mantine-color-default-border) last:border-b-0'
              >
                <div className='min-w-0 flex-1'>
                  <p className='truncate text-sm font-medium'>{subuser.user.username}</p>
                  <p className='truncate text-xs text-(--mantine-color-dimmed)'>{subuser.user.email}</p>
                </div>
                <div className='flex flex-wrap items-center gap-2'>
                  <Badge size='sm' variant='light' color={subuser.on_master ? 'blue' : 'gray'}>
                    {subuser.on_master ? 'On master' : 'Not on master'}
                  </Badge>
                  <Badge size='sm' variant='light' color={subuser.on_split ? 'teal' : 'gray'}>
                    {subuser.on_split ? 'On split' : 'Not on split'}
                  </Badge>
                  <Switch
                    label='Custom'
                    size='sm'
                    checked={subuser.custom}
                    disabled={!canUpdate || savingUser === subuser.user.uuid}
                    onChange={(e) => setCustom(subuser, e.currentTarget.checked)}
                  />
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      <ModalFooter>
        <Button variant='default' onClick={onClose}>
          Close
        </Button>
        {canSync && (
          <Button leftSection={<FontAwesomeIcon icon={faRotate} />} loading={syncing} onClick={syncNow}>
            Sync now
          </Button>
        )}
      </ModalFooter>
    </Modal>
  );
}
