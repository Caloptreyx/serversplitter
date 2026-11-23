import { NumberInput, SimpleGrid, Stack, Text } from '@mantine/core';
import { useEffect, useState } from 'react';
import { httpErrorToHuman } from '@/api/axios.ts';
import Button from '@/elements/buttons/Button.tsx';
import { Modal, ModalFooter } from '@/elements/modals/Modal.tsx';
import { useToast } from '@/providers/ToastProvider.tsx';
import type { Amounts } from '../api/client.ts';
import { type AdminSplitSummary, adminResizeSplit } from '../api/admin.ts';

type FieldKey = 'cpu' | 'memory' | 'disk' | 'allocations' | 'databases' | 'backups' | 'schedules';

const fields: { key: FieldKey; label: string }[] = [
  { key: 'cpu', label: 'CPU (%)' },
  { key: 'memory', label: 'Memory (MB)' },
  { key: 'disk', label: 'Disk (MB)' },
  { key: 'allocations', label: 'Allocations' },
  { key: 'databases', label: 'Databases' },
  { key: 'backups', label: 'Backups' },
  { key: 'schedules', label: 'Schedules' },
];

/** Resizes a split within its master's pool; bounds come from the backend (`max` -1 = unlimited). */
export default function AdminResizeModal({
  split,
  onClose,
  onResized,
}: {
  split: AdminSplitSummary | null;
  onClose: () => void;
  onResized: () => void;
}) {
  const { addToast } = useToast();
  const [values, setValues] = useState<Amounts | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setValues(split ? { ...split.limits } : null);
  }, [split]);

  const outOfRange =
    !!split &&
    !!values &&
    fields.some(({ key }) => values[key] < split.min[key] || (split.max[key] !== -1 && values[key] > split.max[key]));

  const handleSave = async () => {
    if (!split || !values) return;
    try {
      setSaving(true);
      await adminResizeSplit(split.uuid, {
        cpu: values.cpu,
        memory: values.memory,
        disk: values.disk,
        feature_limits: {
          allocations: values.allocations,
          databases: values.databases,
          backups: values.backups,
          schedules: values.schedules,
        },
      });
      addToast(`Resized ${split.name}`, 'success');
      onResized();
      onClose();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal opened={!!split} onClose={onClose} title={split ? `Resize ${split.name}` : 'Resize'} size='lg'>
      {split && values && (
        <Stack gap='md'>
          <Text size='sm' c='dimmed'>
            The difference is taken from or returned to the master&apos;s pool. For CPU, memory and disk 0 means
            unlimited, which is only possible when the master is unlimited too.
          </Text>
          <SimpleGrid cols={{ base: 1, sm: 2 }} spacing='md'>
            {fields.map(({ key, label }) => {
              const min = split.min[key];
              const max = split.max[key];
              return (
                <NumberInput
                  key={key}
                  label={label}
                  description={max === -1 ? `At least ${min}, no upper bound` : `${min} to ${max}`}
                  min={min}
                  max={max === -1 ? undefined : max}
                  allowDecimal={false}
                  value={values[key]}
                  onChange={(val) => setValues({ ...values, [key]: typeof val === 'number' ? val : 0 })}
                />
              );
            })}
          </SimpleGrid>
          <ModalFooter>
            <Button variant='default' onClick={onClose}>
              Cancel
            </Button>
            <Button color='blue' loading={saving} disabled={outOfRange} onClick={handleSave}>
              Resize
            </Button>
          </ModalFooter>
        </Stack>
      )}
    </Modal>
  );
}
