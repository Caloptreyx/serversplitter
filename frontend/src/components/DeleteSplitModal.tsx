import { useEffect, useState } from 'react';
import { httpErrorToHuman } from '@/api/axios.ts';
import Button from '@/elements/buttons/Button.tsx';
import Checkbox from '@/elements/input/Checkbox.tsx';
import TextInput from '@/elements/input/TextInput.tsx';
import { Modal, ModalFooter } from '@/elements/modals/Modal.tsx';
import { useToast } from '@/providers/ToastProvider.tsx';
import { useServerStore } from '@/stores/server.ts';
import { deleteSplit, type MasterPower, type MasterPowerOptions, type SplitEntry } from '../api/client.ts';
import MasterPowerSelect, { initialMasterPower, masterPowerOffered, reportMasterPower } from './MasterPowerSelect.tsx';
import { formatLimit } from './splitterFormat.ts';

export default function DeleteSplitModal({
  split,
  serverUuid,
  masterName,
  onClose,
  masterPower,
  onDeleted,
}: {
  /** The split to delete; `null` keeps the modal closed. */
  split: SplitEntry | null;
  serverUuid: string;
  masterName: string;
  onClose: () => void;
  /** Power actions offered after the delete (`null` = none). */
  masterPower: MasterPowerOptions | null;
  onDeleted: () => void;
}) {
  const { addToast } = useToast();
  const [confirmName, setConfirmName] = useState('');
  const [returnPort, setReturnPort] = useState(true);
  const [power, setPower] = useState<MasterPower>('none');
  const masterState = useServerStore((s) => s.state);
  const powerOffered = masterPowerOffered(masterPower, masterState);
  const [deleting, setDeleting] = useState(false);

  useEffect(() => {
    if (!split) return;
    setConfirmName('');
    setReturnPort(true);
    setPower(initialMasterPower(masterPower));
    setDeleting(false);
  }, [split?.server.uuid]);

  const doDelete = async () => {
    if (!split) return;
    setDeleting(true);

    try {
      const result = await deleteSplit(serverUuid, split.server.uuid, returnPort, powerOffered ? power : 'none');
      reportMasterPower(addToast, `${split.server.name} deleted`, result);
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
          {powerOffered && masterPower && <MasterPowerSelect options={masterPower} value={power} onChange={setPower} />}
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
