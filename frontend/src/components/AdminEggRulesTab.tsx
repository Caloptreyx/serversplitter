import { faEdit, faPlus, faTrash, faTriangleExclamation } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Badge, Card, Group, SimpleGrid, Stack, Table, Text, Title } from '@mantine/core';
import { useMemo, useState } from 'react';
import { httpErrorToHuman } from '@/api/axios.ts';
import Button from '@/elements/buttons/Button.tsx';
import Alert from '@/elements/feedback/Alert.tsx';
import ConfirmationModal from '@/elements/modals/ConfirmationModal.tsx';
import { Modal, ModalFooter } from '@/elements/modals/Modal.tsx';
import { useToast } from '@/providers/ToastProvider.tsx';
import {
  type AdminSettingsResponse,
  createAdminEggRule,
  deleteAdminEggRule,
  type EggRule,
  updateAdminEggRule,
} from '../api/admin.ts';
import EggTreePicker from './EggTreePicker.tsx';

export default function AdminEggRulesTab({
  settings,
  canWrite,
  onChanged,
}: {
  settings: AdminSettingsResponse;
  canWrite: boolean;
  onChanged: () => Promise<void>;
}) {
  const { addToast } = useToast();

  const [isRuleModalOpen, setIsRuleModalOpen] = useState(false);
  const [editingRule, setEditingRule] = useState<EggRule | null>(null);
  const [ruleToDelete, setRuleToDelete] = useState<EggRule | null>(null);
  const [submittingRule, setSubmittingRule] = useState(false);

  const [selectedEggs, setSelectedEggs] = useState<string[]>([]);
  const [selectedAllowedEggs, setSelectedAllowedEggs] = useState<string[]>([]);

  const eggNameMap = useMemo(() => {
    const map = new Map<string, string>();
    for (const egg of settings.eggs) {
      map.set(egg.uuid, `${egg.name} (${egg.nest_name})`);
    }
    return map;
  }, [settings.eggs]);

  const eggLabel = (uuid: string) => eggNameMap.get(uuid) ?? uuid.slice(0, 8);

  // A master egg may be in one rule only; the backend rejects overlaps, this warns before saving.
  const overlappingEggs = useMemo(() => {
    const taken = new Set(
      settings.egg_rules.filter((rule) => rule.id !== editingRule?.id).flatMap((rule) => rule.eggs),
    );
    return selectedEggs.filter((uuid) => taken.has(uuid));
  }, [settings.egg_rules, editingRule, selectedEggs]);

  const handleOpenCreateRule = () => {
    setEditingRule(null);
    setSelectedEggs([]);
    setSelectedAllowedEggs([]);
    setIsRuleModalOpen(true);
  };

  const handleOpenEditRule = (rule: EggRule) => {
    setEditingRule(rule);
    setSelectedEggs(rule.eggs);
    setSelectedAllowedEggs(rule.allowed_eggs);
    setIsRuleModalOpen(true);
  };

  const handleSaveRule = async () => {
    if (selectedEggs.length === 0) {
      addToast('Please select at least one master egg', 'error');
      return;
    }
    if (selectedAllowedEggs.length === 0) {
      addToast('Please select at least one allowed child egg', 'error');
      return;
    }

    try {
      setSubmittingRule(true);
      const payload = { eggs: selectedEggs, allowed_eggs: selectedAllowedEggs };
      if (editingRule) {
        await updateAdminEggRule(editingRule.id, payload);
        addToast('Egg rule updated', 'success');
      } else {
        await createAdminEggRule(payload);
        addToast('Egg rule created', 'success');
      }
      setIsRuleModalOpen(false);
      await onChanged();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setSubmittingRule(false);
    }
  };

  const handleDeleteRule = async () => {
    if (!ruleToDelete) return;
    try {
      await deleteAdminEggRule(ruleToDelete.id);
      addToast('Egg rule deleted', 'success');
      setRuleToDelete(null);
      await onChanged();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    }
  };

  return (
    <>
      <Card padding='lg' radius='md' withBorder>
        <Stack gap='md'>
          <Group justify='space-between' align='center'>
            <div>
              <Title order={4} fw={600}>
                Egg Compatibility Rules
              </Title>
              <Text size='xs' c='dimmed'>
                Define which eggs can be selected when splitting from a given master server egg. If no rule matches a
                server egg, splitting will not allow creating child servers.
              </Text>
            </div>
            {canWrite && (
              <Button
                color='blue'
                size='sm'
                onClick={handleOpenCreateRule}
                leftSection={<FontAwesomeIcon icon={faPlus} />}
              >
                Add Egg Rule
              </Button>
            )}
          </Group>

          {settings.egg_rules.length === 0 ? (
            <div className='py-8 text-center'>
              <Text c='dimmed' size='sm'>
                No egg rules defined yet.{canWrite && ' Click "Add Egg Rule" to permit child egg creation.'}
              </Text>
            </div>
          ) : (
            <Table.ScrollContainer minWidth={600}>
              <Table verticalSpacing='sm' striped highlightOnHover>
                <Table.Thead>
                  <Table.Tr>
                    <Table.Th>Master Eggs</Table.Th>
                    <Table.Th>Allowed Child Eggs</Table.Th>
                    {canWrite && <Table.Th style={{ width: 120 }}>Actions</Table.Th>}
                  </Table.Tr>
                </Table.Thead>
                <Table.Tbody>
                  {settings.egg_rules.map((rule) => (
                    <Table.Tr key={rule.id}>
                      <Table.Td>
                        <Group gap='xs'>
                          {rule.eggs.map((uuid) => (
                            <Badge key={uuid} size='sm' variant='light' color='blue'>
                              {eggLabel(uuid)}
                            </Badge>
                          ))}
                        </Group>
                      </Table.Td>
                      <Table.Td>
                        <Group gap='xs'>
                          {rule.allowed_eggs.map((uuid) => (
                            <Badge key={uuid} size='sm' variant='light' color='teal'>
                              {eggLabel(uuid)}
                            </Badge>
                          ))}
                        </Group>
                      </Table.Td>
                      {canWrite && (
                        <Table.Td>
                          <Group gap='xs'>
                            <Button size='xs' variant='light' color='gray' onClick={() => handleOpenEditRule(rule)}>
                              <FontAwesomeIcon icon={faEdit} />
                            </Button>
                            <Button size='xs' variant='subtle' color='red' onClick={() => setRuleToDelete(rule)}>
                              <FontAwesomeIcon icon={faTrash} />
                            </Button>
                          </Group>
                        </Table.Td>
                      )}
                    </Table.Tr>
                  ))}
                </Table.Tbody>
              </Table>
            </Table.ScrollContainer>
          )}
        </Stack>
      </Card>

      <Modal
        opened={isRuleModalOpen}
        onClose={() => setIsRuleModalOpen(false)}
        title={editingRule ? 'Edit Egg Rule' : 'New Egg Rule'}
        size='xl'
      >
        <Stack gap='md'>
          <SimpleGrid cols={{ base: 1, md: 2 }} spacing='lg'>
            <EggTreePicker
              label='Master Eggs'
              description='Servers using any of these eggs get this rule'
              eggs={settings.eggs}
              value={selectedEggs}
              onChange={setSelectedEggs}
            />

            <EggTreePicker
              label='Allowed Child Eggs'
              description='Eggs users can pick when creating a split'
              eggs={settings.eggs}
              value={selectedAllowedEggs}
              onChange={setSelectedAllowedEggs}
              actions={
                <Button
                  size='xs'
                  variant='light'
                  disabled={selectedEggs.length === 0}
                  onClick={() => setSelectedAllowedEggs(selectedEggs)}
                >
                  Same as master eggs
                </Button>
              }
            />
          </SimpleGrid>

          {overlappingEggs.length > 0 && (
            <Alert
              color='orange'
              title='Master egg already in another rule'
              icon={<FontAwesomeIcon icon={faTriangleExclamation} />}
            >
              {overlappingEggs.map(eggLabel).join(', ')} {overlappingEggs.length === 1 ? 'is' : 'are'} already a master
              egg of another rule. A master egg can belong to one rule only, so saving will be rejected until you remove{' '}
              {overlappingEggs.length === 1 ? 'it' : 'them'} here or from the other rule.
            </Alert>
          )}

          <ModalFooter>
            <Button variant='default' onClick={() => setIsRuleModalOpen(false)}>
              Cancel
            </Button>
            <Button color='blue' loading={submittingRule} onClick={handleSaveRule}>
              Save Rule
            </Button>
          </ModalFooter>
        </Stack>
      </Modal>

      <ConfirmationModal
        opened={!!ruleToDelete}
        onClose={() => setRuleToDelete(null)}
        title='Delete Egg Rule'
        confirm='Delete Rule'
        confirmColor='red'
        onConfirmed={handleDeleteRule}
      >
        <Text size='sm'>
          Are you sure you want to delete this egg rule? Users with servers matching these master eggs will no longer be
          able to create splits for these eggs.
        </Text>
      </ConfirmationModal>
    </>
  );
}
