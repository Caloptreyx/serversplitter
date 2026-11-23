import { useEffect, useState } from 'react';
import { httpErrorToHuman } from '@/api/axios.ts';
import Button from '@/elements/buttons/Button.tsx';
import Checkbox from '@/elements/input/Checkbox.tsx';
import TextInput from '@/elements/input/TextInput.tsx';
import { Modal, ModalFooter } from '@/elements/modals/Modal.tsx';
import { useToast } from '@/providers/ToastProvider.tsx';
import { deleteSplit, type SplitEntry } from '../api/client.ts';
import { formatLimit } from './splitterFormat.ts';

export default function DeleteSplitModal({
  split,
  serverUuid,
  masterName,
  onClose,
  onDeleted,
}: {
  /** The split to delete; `null` keeps the modal closed. */
  split: SplitEntry | null;
  serverUuid: string;
  masterName: string;
  onClose: () => void;
  onDeleted: () => void;
}) {
  const { addToast } = useToast();
  const [confirmName, setConfirmName] = useState('');
  const [returnPort, setReturnPort] = useState(true);
  const [deleting, setDeleting] = useState(false);

  useEffect(() => {
    if (!split) return;
    setConfirmName('');
    setReturnPort(true);
    setDeleting(false);
  }, [split?.server.uuid]);

  const doDelete = async () => {
    if (!split) return;
    setDeleting(true);

    try {
      await deleteSplit(serverUuid, split.server.uuid, returnPort);
      addToast(`${split.server.name} deleted`, 'success');
      onClose();
      onDeleted();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setDeleting(false);
    }
  };

  const ledger = split?.ledger;

  return (
    <Modal opened={!!split} onClose={onClose} title={split ? `Delete ${split.server.name}` : 'Delete split'}>
      {split && ledger && (
        <div className='flex flex-col gap-3 text-sm'>
          <p>
            This permanently deletes the split with all its files, databases and backups. Its resources go back to{' '}
            <span className='font-semibold'>{masterName}</span>: {formatLimit('cpu', ledger.cpu)} CPU,{' '}
            {formatLimit('memory', ledger.memory)} memory, {formatLimit('disk', ledger.disk)} disk, and its database,
            backup and schedule limits.
          </p>
          <Checkbox
            label={`Give its ports back to ${masterName}`}
            description={
              split.port_from_master ? `Its main port came from ${masterName}.` : `Up to ${masterName}'s port limit.`
            }
            checked={returnPort}
            onChange={(e) => setReturnPort(e.currentTarget.checked)}
          />
          <TextInput
            label={
              <>
                Type <span className='font-semibold'>{split.server.name}</span> to confirm
              </>
            }
            value={confirmName}
            onChange={(e) => setConfirmName(e.currentTarget.value)}
            autoComplete='off'
            data-autofocus
          />
        </div>
      )}

      <ModalFooter>
        <Button variant='default' onClick={onClose}>
          Cancel
        </Button>
        <Button color='red' loading={deleting} disabled={confirmName !== split?.server.name} onClick={doDelete}>
          Delete split
        </Button>
      </ModalFooter>
    </Modal>
  );
}
