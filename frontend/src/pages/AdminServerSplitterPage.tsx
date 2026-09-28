import { faDiscord } from '@fortawesome/free-brands-svg-icons';
import { faCog, faEdit, faEgg, faPlus, faSave, faTrash } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import {
  Badge,
  Card,
  Divider,
  Group,
  Button as MantineButton,
  NumberInput,
  SimpleGrid,
  Stack,
  Switch,
  Table,
  Tabs,
  Text,
  Title,
} from '@mantine/core';
import { useEffect, useMemo, useState } from 'react';
import { httpErrorToHuman } from '@/api/axios.ts';
import AlertError from '@/elements/alerts/AlertError.tsx';
import Button from '@/elements/buttons/Button.tsx';
import Spinner from '@/elements/feedback/Spinner.tsx';
import ConfirmationModal from '@/elements/modals/ConfirmationModal.tsx';
import { Modal, ModalFooter } from '@/elements/modals/Modal.tsx';
import { useAdminCan } from '@/plugins/usePermissions.ts';
import { useToast } from '@/providers/ToastProvider.tsx';
import {
  type AdminSettingsResponse,
  createAdminEggRule,
  deleteAdminEggRule,
  type EggRule,
  getAdminSplitterSettings,
  updateAdminEggRule,
  updateAdminSplitterSettings,
} from '../api/client.ts';
import EggTreePicker from '../components/EggTreePicker.tsx';

export default function AdminServerSplitterPage() {
  const { addToast } = useToast();
  const canWrite = useAdminCan('extensions.splitter.write');

  const [loading, setLoading] = useState(true);
  const [savingSettings, setSavingSettings] = useState(false);
  const [settings, setSettings] = useState<AdminSettingsResponse | null>(null);
  const [loadError, setLoadError] = useState('');

  // General Settings Form
  const [reservedCpu, setReservedCpu] = useState<number>(0);
  const [reservedMemory, setReservedMemory] = useState<number>(0);
  const [reservedDisk, setReservedDisk] = useState<number>(0);
  const [includeDiskUsage, setIncludeDiskUsage] = useState(false);
  const [displayReservedLimits, setDisplayReservedLimits] = useState(true);
  const [defaultSplits, setDefaultSplits] = useState<number>(0);

  // Egg Rules Modals State
  const [isRuleModalOpen, setIsRuleModalOpen] = useState(false);
  const [editingRule, setEditingRule] = useState<EggRule | null>(null);
  const [ruleToDelete, setRuleToDelete] = useState<EggRule | null>(null);
  const [submittingRule, setSubmittingRule] = useState(false);

  // Rule Form State
  const [selectedEggs, setSelectedEggs] = useState<string[]>([]);
  const [selectedAllowedEggs, setSelectedAllowedEggs] = useState<string[]>([]);

  const loadSettings = async () => {
    try {
      setLoading(true);
      setLoadError('');
      const data = await getAdminSplitterSettings();
      setSettings(data);
      setReservedCpu(data.reserved_cpu);
      setReservedMemory(data.reserved_memory);
      setReservedDisk(data.reserved_disk);
      setIncludeDiskUsage(data.include_disk_usage);
      setDisplayReservedLimits(data.display_reserved_limits);
      setDefaultSplits(data.default_splits);
    } catch (err) {
      setLoadError(httpErrorToHuman(err));
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadSettings();
  }, []);

  const eggNameMap = useMemo(() => {
    const map = new Map<string, { name: string; nestName: string }>();
    if (settings?.eggs) {
      for (const egg of settings.eggs) {
        map.set(egg.uuid, { name: egg.name, nestName: egg.nest_name });
      }
    }
    return map;
  }, [settings?.eggs]);

  const handleSaveSettings = async () => {
    try {
      setSavingSettings(true);
      await updateAdminSplitterSettings({
        reserved_cpu: Number(reservedCpu),
        reserved_memory: Number(reservedMemory),
        reserved_disk: Number(reservedDisk),
        include_disk_usage: includeDiskUsage,
        display_reserved_limits: displayReservedLimits,
        default_splits: Number(defaultSplits),
      });
      addToast('Settings saved successfully', 'success');
      await loadSettings();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setSavingSettings(false);
    }
  };

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
      addToast('Please select at least one parent egg', 'error');
      return;
    }
    if (selectedAllowedEggs.length === 0) {
      addToast('Please select at least one allowed child egg', 'error');
      return;
    }

    try {
      setSubmittingRule(true);
      if (editingRule) {
        await updateAdminEggRule(editingRule.id, {
          eggs: selectedEggs,
          allowed_eggs: selectedAllowedEggs,
        });
        addToast('Egg rule updated', 'success');
      } else {
        await createAdminEggRule({
          eggs: selectedEggs,
          allowed_eggs: selectedAllowedEggs,
        });
        addToast('Egg rule created', 'success');
      }
      setIsRuleModalOpen(false);
      await loadSettings();
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
      await loadSettings();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    }
  };

  const header = (
    <Group justify='space-between' align='center'>
      <div>
        <Title order={3} fw={700}>
          Server Splitter Configuration
        </Title>
        <Text size='sm' c='dimmed'>
          Configure master resource reservations and egg permission rules.
        </Text>
      </div>
      <MantineButton
        component='a'
        href='https://discord.gg/4qjMWU7S8x'
        target='_blank'
        rel='noopener noreferrer'
        variant='default'
        leftSection={<FontAwesomeIcon icon={faDiscord} />}
      >
        Support & feature requests
      </MantineButton>
    </Group>
  );

  if (loading && !settings) {
    return (
      <div className='flex justify-center items-center py-24'>
        <Spinner />
      </div>
    );
  }

  if (!settings) {
    // Never render the editable form with placeholder values: saving it would overwrite real settings.
    return (
      <Stack gap='lg'>
        {header}
        {loadError && <AlertError error={loadError} setError={setLoadError} />}
        <Group>
          <Button onClick={loadSettings}>Retry</Button>
        </Group>
      </Stack>
    );
  }

  return (
    <Stack gap='lg'>
      {header}

      <Tabs defaultValue='general'>
        <Tabs.List mb='md'>
          <Tabs.Tab value='general' leftSection={<FontAwesomeIcon icon={faCog} />}>
            General Settings
          </Tabs.Tab>
          <Tabs.Tab
            value='eggRules'
            leftSection={<FontAwesomeIcon icon={faEgg} />}
            rightSection={
              <Badge size='xs' variant='filled'>
                {settings?.egg_rules.length ?? 0}
              </Badge>
            }
          >
            Egg Rules
          </Tabs.Tab>
        </Tabs.List>

        {/* GENERAL SETTINGS TAB */}
        <Tabs.Panel value='general'>
          <Card padding='lg' radius='md' withBorder>
            <Stack gap='md'>
              <Title order={4} fw={600}>
                Master Server Reserved Limits
              </Title>
              <Text size='xs' c='dimmed'>
                Resources reserved exclusively for the master server that cannot be allocated to any child server
                splits.
              </Text>

              <SimpleGrid cols={{ base: 1, sm: 3 }} spacing='md'>
                <NumberInput
                  disabled={!canWrite}
                  label='Reserved CPU (%)'
                  description='Minimum CPU % kept by parent'
                  min={0}
                  value={reservedCpu}
                  onChange={(val) => setReservedCpu(typeof val === 'number' ? val : 0)}
                />
                <NumberInput
                  disabled={!canWrite}
                  label='Reserved Memory (MB)'
                  description='Minimum RAM (MB) kept by parent'
                  min={0}
                  value={reservedMemory}
                  onChange={(val) => setReservedMemory(typeof val === 'number' ? val : 0)}
                />
                <NumberInput
                  disabled={!canWrite}
                  label='Reserved Disk (MB)'
                  description='Minimum Disk (MB) kept by parent'
                  min={0}
                  value={reservedDisk}
                  onChange={(val) => setReservedDisk(typeof val === 'number' ? val : 0)}
                />
              </SimpleGrid>

              <Divider my='sm' />

              <Title order={4} fw={600}>
                New Servers
              </Title>

              <NumberInput
                disabled={!canWrite}
                label='Default Split Limit'
                description='Split limit applied to new servers unless overridden.'
                min={0}
                w={220}
                value={defaultSplits}
                onChange={(val) => setDefaultSplits(typeof val === 'number' ? val : 0)}
              />

              <Divider my='sm' />

              <Title order={4} fw={600}>
                Display & Calculation Options
              </Title>

              <Switch
                disabled={!canWrite}
                label='Include master server live disk usage'
                description='Factor in current disk storage consumed when calculating remaining disk pool'
                checked={includeDiskUsage}
                onChange={(e) => setIncludeDiskUsage(e.currentTarget.checked)}
              />

              <Switch
                disabled={!canWrite}
                label='Display reserved limits on client interface'
                description='Show reserved values to users on the client splitter page'
                checked={displayReservedLimits}
                onChange={(e) => setDisplayReservedLimits(e.currentTarget.checked)}
              />

              {canWrite && (
                <Group justify='flex-end' mt='md'>
                  <Button
                    color='blue'
                    loading={savingSettings}
                    onClick={handleSaveSettings}
                    leftSection={<FontAwesomeIcon icon={faSave} />}
                  >
                    Save Settings
                  </Button>
                </Group>
              )}
            </Stack>
          </Card>
        </Tabs.Panel>

        {/* EGG RULES TAB */}
        <Tabs.Panel value='eggRules'>
          <Card padding='lg' radius='md' withBorder>
            <Stack gap='md'>
              <Group justify='space-between' align='center'>
                <div>
                  <Title order={4} fw={600}>
                    Egg Compatibility Rules
                  </Title>
                  <Text size='xs' c='dimmed'>
                    Define which eggs can be selected when splitting from a given parent server egg. If no rule matches
                    a server egg, splitting will not allow creating child servers.
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

              {settings?.egg_rules.length === 0 ? (
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
                      {settings?.egg_rules.map((rule) => (
                        <Table.Tr key={rule.id}>
                          <Table.Td>
                            <Group gap='xs'>
                              {rule.eggs.map((uuid) => {
                                const info = eggNameMap.get(uuid);
                                return (
                                  <Badge key={uuid} size='sm' variant='light' color='blue'>
                                    {info ? `${info.name} (${info.nestName})` : uuid.slice(0, 8)}
                                  </Badge>
                                );
                              })}
                            </Group>
                          </Table.Td>
                          <Table.Td>
                            <Group gap='xs'>
                              {rule.allowed_eggs.map((uuid) => {
                                const info = eggNameMap.get(uuid);
                                return (
                                  <Badge key={uuid} size='sm' variant='light' color='teal'>
                                    {info ? `${info.name} (${info.nestName})` : uuid.slice(0, 8)}
                                  </Badge>
                                );
                              })}
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
        </Tabs.Panel>
      </Tabs>

      {/* ADD / EDIT EGG RULE MODAL */}
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
              eggs={settings?.eggs ?? []}
              value={selectedEggs}
              onChange={setSelectedEggs}
            />

            <EggTreePicker
              label='Allowed Child Eggs'
              description='Eggs users can pick when creating a split'
              eggs={settings?.eggs ?? []}
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

      {/* DELETE RULE CONFIRMATION */}
      <ConfirmationModal
        opened={!!ruleToDelete}
        onClose={() => setRuleToDelete(null)}
        title='Delete Egg Rule'
        confirm='Delete Rule'
        confirmColor='red'
        onConfirmed={handleDeleteRule}
      >
        <Text size='sm'>
          Are you sure you want to delete this egg rule? Users with servers matching these parent eggs will no longer be
          able to create splits for these eggs.
        </Text>
      </ConfirmationModal>
    </Stack>
  );
}
