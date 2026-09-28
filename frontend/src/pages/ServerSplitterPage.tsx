import {
  faArrowRight,
  faExclamationTriangle,
  faHdd,
  faInfoCircle,
  faMemory,
  faMicrochip,
  faPlus,
  faServer,
} from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import {
  Alert,
  Card,
  Group,
  NumberInput,
  Select,
  SimpleGrid,
  Stack,
  Switch,
  Text,
  TextInput,
  Title,
} from '@mantine/core';
import { useEffect, useMemo, useState } from 'react';
import { NavLink as Link } from 'react-router';
import { httpErrorToHuman } from '@/api/axios.ts';
import Button from '@/elements/buttons/Button.tsx';
import ServerContentContainer from '@/elements/containers/ServerContentContainer.tsx';
import StatCard from '@/elements/data-display/StatCard.tsx';
import Spinner from '@/elements/feedback/Spinner.tsx';
import ConfirmationModal from '@/elements/modals/ConfirmationModal.tsx';
import { Modal, ModalFooter } from '@/elements/modals/Modal.tsx';
import { useServerCan } from '@/plugins/usePermissions.ts';
import { useToast } from '@/providers/ToastProvider.tsx';
import { useServerStore } from '@/stores/server.ts';
import {
  createSplit,
  deleteSplit,
  getClientSplitter,
  getClientSplitterNests,
  type NestEggItem,
  type ResourcesData,
  type Server,
  type SplitterClientIndex,
  syncSubusers,
  updateSplit,
} from '../api/client.ts';
import SplitCard from '../components/SplitCard.tsx';

function formatBytes(mb: number): string {
  if (mb >= 1024) {
    return `${(mb / 1024).toFixed(1)} GB`;
  }
  return `${mb} MB`;
}

/** StatCard props for a pool resource; `-1` means the master's resource is unlimited. */
function poolStat(remaining: number, total: number, format: (value: number) => string) {
  if (remaining === -1) return { value: 'Unlimited' };
  return { value: format(remaining), limit: format(total), progress: total - remaining, total };
}

type PoolResource = 'cpu' | 'memory' | 'disk';

interface Bounds {
  min: number;
  /** `undefined` when the master is unlimited for this resource. */
  max: number | undefined;
}

/**
 * Bounds the backend accepts for a split's cpu/memory/disk. `current` is the split's existing value
 * when resizing (its share returns to the pool, and keeping it is always accepted), `undefined` when creating.
 */
function resourceBounds(resources: ResourcesData, key: PoolResource, current?: number): Bounds {
  const remaining = resources.remaining[key];
  // A limited master requires each split to be limited too; an unlimited one allows 0 (= unlimited).
  const minimum = resources.total[key] > 0 ? Math.max(resources.reserved[key], 1) : 0;
  const min = current === undefined ? minimum : Math.min(minimum, current);
  const max = remaining === -1 ? undefined : (current ?? 0) + remaining;
  return { min, max };
}

function defaultWithin(preferred: number, { min, max }: Bounds): number {
  return Math.max(min, max === undefined ? preferred : Math.min(preferred, max));
}

function boundsText(prefix: string, { min, max }: Bounds, format: (value: number) => string): string {
  const upper = max === undefined ? 'Unlimited' : format(max);
  return min === 0 ? `${prefix}: ${upper} (0 = unlimited)` : `${prefix}: ${upper}`;
}

const formatPercent = (value: number) => `${value}%`;

export default function ServerSplitterPage() {
  const currentServer = useServerStore((state) => state.server);
  const { addToast } = useToast();

  const [loading, setLoading] = useState(true);
  const [data, setData] = useState<SplitterClientIndex | null>(null);
  const [nestsData, setNestsData] = useState<Record<string, NestEggItem[]>>({});

  // Modals state
  const [isCreateOpen, setIsCreateOpen] = useState(false);
  const [isEditOpen, setIsEditOpen] = useState(false);
  const [serverToDelete, setServerToDelete] = useState<Server | null>(null);
  const [editingSubserver, setEditingSubserver] = useState<Server | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [syncingSubusers, setSyncingSubusers] = useState<string | null>(null);

  // Form states for Create
  const [createName, setCreateName] = useState('');
  const [createDescription, setCreateDescription] = useState('');
  const [createEggUuid, setCreateEggUuid] = useState<string | null>(null);
  const [createCpu, setCreateCpu] = useState<number>(100);
  const [createMemory, setCreateMemory] = useState<number>(1024);
  const [createDisk, setCreateDisk] = useState<number>(2048);
  const [createAllocations, setCreateAllocations] = useState<number>(1);
  const [createDatabases, setCreateDatabases] = useState<number>(0);
  const [createBackups, setCreateBackups] = useState<number>(0);
  const [createSchedules, setCreateSchedules] = useState<number>(0);
  const [createSyncSubusers, setCreateSyncSubusers] = useState(true);

  // Form states for Edit
  const [editName, setEditName] = useState('');
  const [editDescription, setEditDescription] = useState('');
  const [editCpu, setEditCpu] = useState<number>(100);
  const [editMemory, setEditMemory] = useState<number>(1024);
  const [editDisk, setEditDisk] = useState<number>(2048);
  const [editAllocations, setEditAllocations] = useState<number>(1);
  const [editDatabases, setEditDatabases] = useState<number>(0);
  const [editBackups, setEditBackups] = useState<number>(0);
  const [editSchedules, setEditSchedules] = useState<number>(0);

  const loadSplitterData = async () => {
    if (!currentServer?.uuid) return;
    try {
      setLoading(true);
      const res = await getClientSplitter(currentServer.uuid);
      setData(res);
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setLoading(false);
    }
  };

  const loadNests = async () => {
    if (!currentServer?.uuid) return;
    try {
      const nests = await getClientSplitterNests(currentServer.uuid);
      setNestsData(nests);
      // Pick first egg by default if available
      const firstNest = Object.values(nests)[0];
      if (firstNest && firstNest[0]) {
        setCreateEggUuid(firstNest[0].uuid);
      }
    } catch {
      // Ignored
    }
  };

  useEffect(() => {
    loadSplitterData();
    loadNests();
  }, [currentServer?.uuid]);

  // Available eggs list for select dropdown
  const eggSelectOptions = useMemo(() => {
    const options: { group: string; items: { value: string; label: string }[] }[] = [];
    for (const [nestName, eggs] of Object.entries(nestsData)) {
      options.push({
        group: nestName,
        items: eggs.map((e) => ({
          value: e.uuid,
          label: e.name,
        })),
      });
    }
    return options;
  }, [nestsData]);

  const canCreate = useServerCan('splitter.create');

  const handleOpenCreate = () => {
    const resources = data?.resources;
    if (!resources) return;

    setCreateName('');
    setCreateDescription('');
    setCreateCpu(defaultWithin(100, resourceBounds(resources, 'cpu')));
    setCreateMemory(defaultWithin(1024, resourceBounds(resources, 'memory')));
    setCreateDisk(defaultWithin(2048, resourceBounds(resources, 'disk')));
    setCreateAllocations(1);
    setCreateDatabases(0);
    setCreateBackups(0);
    setCreateSchedules(0);
    setCreateSyncSubusers(true);
    setIsCreateOpen(true);
  };

  const handleCreateSubmit = async () => {
    if (!currentServer?.uuid || !createEggUuid) return;
    if (!createName.trim()) {
      addToast('Server name is required', 'error');
      return;
    }

    try {
      setSubmitting(true);
      await createSplit(currentServer.uuid, {
        name: createName.trim(),
        description: createDescription.trim() || undefined,
        egg_uuid: createEggUuid,
        cpu: Number(createCpu),
        memory: Number(createMemory),
        disk: Number(createDisk),
        feature_limits: {
          allocations: Number(createAllocations),
          databases: Number(createDatabases),
          backups: Number(createBackups),
          schedules: Number(createSchedules),
        },
        sync_subusers: createSyncSubusers,
      });

      addToast('Child server created successfully', 'success');
      setIsCreateOpen(false);
      await loadSplitterData();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setSubmitting(false);
    }
  };

  const handleOpenEdit = (subserver: Server) => {
    setEditingSubserver(subserver);
    setEditName(subserver.name);
    setEditDescription(subserver.description || '');
    setEditCpu(subserver.limits.cpu);
    setEditMemory(subserver.limits.memory);
    setEditDisk(subserver.limits.disk);
    setEditAllocations(subserver.featureLimits.allocations);
    setEditDatabases(subserver.featureLimits.databases);
    setEditBackups(subserver.featureLimits.backups);
    setEditSchedules(subserver.featureLimits.schedules);
    setIsEditOpen(true);
  };

  const handleEditSubmit = async () => {
    if (!currentServer?.uuid || !editingSubserver) return;
    try {
      setSubmitting(true);
      await updateSplit(currentServer.uuid, editingSubserver.uuid, {
        name: editName.trim() || undefined,
        // Always sent: an empty string clears the description.
        description: editDescription.trim(),
        cpu: Number(editCpu),
        memory: Number(editMemory),
        disk: Number(editDisk),
        feature_limits: {
          allocations: Number(editAllocations),
          databases: Number(editDatabases),
          backups: Number(editBackups),
          schedules: Number(editSchedules),
        },
      });

      addToast('Server limits updated successfully', 'success');
      setIsEditOpen(false);
      setEditingSubserver(null);
      await loadSplitterData();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setSubmitting(false);
    }
  };

  const handleDeleteConfirmed = async () => {
    if (!currentServer?.uuid || !serverToDelete) return;
    try {
      await deleteSplit(currentServer.uuid, serverToDelete.uuid);
      addToast('Child server deleted successfully', 'success');
      setServerToDelete(null);
      await loadSplitterData();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    }
  };

  const handleSyncSubusers = async (subserver: Server) => {
    if (!currentServer?.uuid) return;
    try {
      setSyncingSubusers(subserver.uuid);
      await syncSubusers(currentServer.uuid, subserver.uuid);
      addToast(`Subusers synced to ${subserver.name}`, 'success');
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setSyncingSubusers(null);
    }
  };

  if (loading && !data) {
    return (
      <ServerContentContainer title='Server Splitter'>
        <div className='flex justify-center items-center py-24'>
          <Spinner />
        </div>
      </ServerContentContainer>
    );
  }

  const resources = data?.resources ?? null;
  const pool = resources?.remaining_display;
  const remaining = resources?.remaining;
  const total = resources?.total;
  const subservers = data?.subservers ?? [];
  const parentServer = data?.parent ?? null;
  const maxSplits = total?.feature_limits.splits ?? 0;
  const canCreateMore = canCreate && maxSplits > 0 && subservers.length < maxSplits;

  const createBounds = resources
    ? {
        cpu: resourceBounds(resources, 'cpu'),
        memory: resourceBounds(resources, 'memory'),
        disk: resourceBounds(resources, 'disk'),
      }
    : null;
  const editBounds =
    resources && editingSubserver
      ? {
          cpu: resourceBounds(resources, 'cpu', editingSubserver.limits.cpu),
          memory: resourceBounds(resources, 'memory', editingSubserver.limits.memory),
          disk: resourceBounds(resources, 'disk', editingSubserver.limits.disk),
          allocations:
            editingSubserver.featureLimits.allocations +
            resources.remaining.feature_limits.allocations -
            (resources.transferable_allocation ? 1 : 0),
          databases: editingSubserver.featureLimits.databases + resources.remaining.feature_limits.databases,
          backups: editingSubserver.featureLimits.backups + resources.remaining.feature_limits.backups,
          schedules: editingSubserver.featureLimits.schedules + resources.remaining.feature_limits.schedules,
        }
      : null;

  return (
    <ServerContentContainer
      title='Server Splitter'
      subtitle={
        parentServer
          ? `Child server of ${parentServer.name}`
          : maxSplits > 0
            ? `Splits: ${subservers.length} of ${maxSplits} used`
            : 'Splits: Disabled for this server'
      }
      contentRight={
        canCreateMore ? (
          <Button onClick={handleOpenCreate} leftSection={<FontAwesomeIcon icon={faPlus} />}>
            Create Split
          </Button>
        ) : undefined
      }
    >
      <Stack gap='lg'>
        {/* Child Server Notice */}
        {parentServer && (
          <Alert icon={<FontAwesomeIcon icon={faInfoCircle} />} title='Child Server' color='blue' radius='md'>
            <Group justify='space-between' align='center'>
              <Text size='sm'>
                This server is a child instance split from master server{' '}
                <Text span fw={700}>
                  {parentServer.name}
                </Text>
                . Its resources belong to the master server&apos;s pool, and splits are managed from the master server.
              </Text>
              <Link to={`/server/${parentServer.uuid}/splitter`}>
                <Button size='xs' variant='light' rightSection={<FontAwesomeIcon icon={faArrowRight} />}>
                  Go to Master Server
                </Button>
              </Link>
            </Group>
          </Alert>
        )}

        {/* Max splits reached or disabled notice */}
        {maxSplits === 0 && !parentServer && (
          <Alert
            icon={<FontAwesomeIcon icon={faExclamationTriangle} />}
            title='Splitting Disabled'
            color='yellow'
            radius='md'
          >
            Server splitting is not enabled for this server. Contact an administrator to allocate splits to your server.
          </Alert>
        )}

        {maxSplits > 0 && subservers.length >= maxSplits && (
          <Alert icon={<FontAwesomeIcon icon={faInfoCircle} />} title='Maximum Splits Reached' color='gray' radius='md'>
            You have reached the maximum allowed child servers ({maxSplits}). Delete an existing child server to reclaim
            split capacity.
          </Alert>
        )}

        {/* Resource pool: what is still free to hand out to new splits */}
        {pool && total && (
          <div className='grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-4 gap-4'>
            <StatCard
              icon={faMicrochip}
              label='CPU available'
              {...poolStat(pool.cpu, total.cpu, (value) => `${value}%`)}
            />
            <StatCard icon={faMemory} label='Memory available' {...poolStat(pool.memory, total.memory, formatBytes)} />
            <StatCard icon={faHdd} label='Disk available' {...poolStat(pool.disk, total.disk, formatBytes)} />
            <StatCard
              icon={faServer}
              label='Splits'
              value={String(subservers.length)}
              limit={String(maxSplits)}
              progress={subservers.length}
              total={maxSplits}
            />
          </div>
        )}

        {!parentServer && (
          <div>
            <Title order={4} mb='md' fw={600}>
              Splits
            </Title>

            {subservers.length === 0 ? (
              <Card padding='xl' radius='md' withBorder>
                <Stack align='center' gap='xs' py='md'>
                  <Text fw={600}>No splits yet</Text>
                  <Text size='sm' c='dimmed' maw={440} ta='center'>
                    Give part of this server&apos;s resources to a separate server with its own console, files and egg.
                  </Text>
                  {canCreateMore && (
                    <Button mt='sm' onClick={handleOpenCreate} leftSection={<FontAwesomeIcon icon={faPlus} />}>
                      Create Split
                    </Button>
                  )}
                </Stack>
              </Card>
            ) : (
              <div className='grid grid-cols-1 lg:grid-cols-2 gap-4'>
                {subservers.map((sub) => (
                  <SplitCard
                    key={sub.uuid}
                    server={sub}
                    syncing={syncingSubusers === sub.uuid}
                    onResize={() => handleOpenEdit(sub)}
                    onSyncUsers={() => handleSyncSubusers(sub)}
                    onDelete={() => setServerToDelete(sub)}
                  />
                ))}
              </div>
            )}
          </div>
        )}
      </Stack>

      {/* CREATE SPLIT MODAL */}
      <Modal opened={isCreateOpen} onClose={() => setIsCreateOpen(false)} title='Create Child Server' size='lg'>
        <Stack gap='md'>
          <TextInput
            label='Server Name'
            placeholder='e.g. Lobby Proxy'
            required
            maxLength={255}
            value={createName}
            onChange={(e) => setCreateName(e.currentTarget.value)}
          />

          <TextInput
            label='Description'
            placeholder='Optional description'
            maxLength={1024}
            value={createDescription}
            onChange={(e) => setCreateDescription(e.currentTarget.value)}
          />

          {eggSelectOptions.length > 0 ? (
            <Select
              label='Server Template (Egg)'
              placeholder='Select Egg'
              required
              data={eggSelectOptions}
              value={createEggUuid}
              onChange={(val) => setCreateEggUuid(val ? String(val) : null)}
              searchable
            />
          ) : (
            <Alert color='yellow' icon={<FontAwesomeIcon icon={faExclamationTriangle} />}>
              No allowed eggs configured for splitting. Contact an administrator.
            </Alert>
          )}

          <Title order={5} mt='xs'>
            Resource Allocation
          </Title>

          {createBounds && remaining && (
            <>
              <SimpleGrid cols={{ base: 1, sm: 3 }} spacing='md'>
                <NumberInput
                  label='CPU Limit (%)'
                  required
                  min={createBounds.cpu.min}
                  max={createBounds.cpu.max}
                  value={createCpu}
                  onChange={(val) => setCreateCpu(typeof val === 'number' ? val : createBounds.cpu.min)}
                  description={boundsText('Available', createBounds.cpu, formatPercent)}
                />

                <NumberInput
                  label='Memory (MB)'
                  required
                  min={createBounds.memory.min}
                  max={createBounds.memory.max}
                  value={createMemory}
                  onChange={(val) => setCreateMemory(typeof val === 'number' ? val : createBounds.memory.min)}
                  description={boundsText('Available', createBounds.memory, formatBytes)}
                />

                <NumberInput
                  label='Disk (MB)'
                  required
                  min={createBounds.disk.min}
                  max={createBounds.disk.max}
                  value={createDisk}
                  onChange={(val) => setCreateDisk(typeof val === 'number' ? val : createBounds.disk.min)}
                  description={boundsText('Available', createBounds.disk, formatBytes)}
                />
              </SimpleGrid>

              <Title order={5} mt='xs'>
                Feature Limits
              </Title>

              <SimpleGrid cols={{ base: 2, sm: 4 }} spacing='md'>
                <NumberInput
                  label='Allocations'
                  min={1}
                  max={remaining.feature_limits.allocations}
                  value={createAllocations}
                  onChange={(val) => setCreateAllocations(typeof val === 'number' ? val : 1)}
                  description={`Available: ${remaining.feature_limits.allocations}`}
                />
                <NumberInput
                  label='Databases'
                  min={0}
                  max={remaining.feature_limits.databases}
                  value={createDatabases}
                  onChange={(val) => setCreateDatabases(typeof val === 'number' ? val : 0)}
                  description={`Available: ${remaining.feature_limits.databases}`}
                />
                <NumberInput
                  label='Backups'
                  min={0}
                  max={remaining.feature_limits.backups}
                  value={createBackups}
                  onChange={(val) => setCreateBackups(typeof val === 'number' ? val : 0)}
                  description={`Available: ${remaining.feature_limits.backups}`}
                />
                <NumberInput
                  label='Schedules'
                  min={0}
                  max={remaining.feature_limits.schedules}
                  value={createSchedules}
                  onChange={(val) => setCreateSchedules(typeof val === 'number' ? val : 0)}
                  description={`Available: ${remaining.feature_limits.schedules}`}
                />
              </SimpleGrid>
            </>
          )}

          <Switch
            label='Sync subusers and permissions to child server'
            checked={createSyncSubusers}
            onChange={(e) => setCreateSyncSubusers(e.currentTarget.checked)}
            mt='xs'
          />

          <ModalFooter>
            <Button variant='default' onClick={() => setIsCreateOpen(false)}>
              Cancel
            </Button>
            <Button color='blue' loading={submitting} onClick={handleCreateSubmit} disabled={!createEggUuid}>
              Create Server
            </Button>
          </ModalFooter>
        </Stack>
      </Modal>

      {/* EDIT / RESIZE MODAL */}
      <Modal
        opened={isEditOpen}
        onClose={() => setIsEditOpen(false)}
        title={`Resize ${editingSubserver?.name ?? 'Child Server'}`}
        size='lg'
      >
        <Stack gap='md'>
          <TextInput
            label='Server Name'
            maxLength={255}
            value={editName}
            onChange={(e) => setEditName(e.currentTarget.value)}
          />

          <TextInput
            label='Description'
            maxLength={1024}
            value={editDescription}
            onChange={(e) => setEditDescription(e.currentTarget.value)}
          />

          {editBounds && (
            <>
              <Title order={5} mt='xs'>
                Resource Limits
              </Title>

              <SimpleGrid cols={{ base: 1, sm: 3 }} spacing='md'>
                <NumberInput
                  label='CPU (%)'
                  required
                  min={editBounds.cpu.min}
                  max={editBounds.cpu.max}
                  value={editCpu}
                  onChange={(val) => setEditCpu(typeof val === 'number' ? val : editBounds.cpu.min)}
                  description={boundsText('Max', editBounds.cpu, formatPercent)}
                />

                <NumberInput
                  label='Memory (MB)'
                  required
                  min={editBounds.memory.min}
                  max={editBounds.memory.max}
                  value={editMemory}
                  onChange={(val) => setEditMemory(typeof val === 'number' ? val : editBounds.memory.min)}
                  description={boundsText('Max', editBounds.memory, formatBytes)}
                />

                <NumberInput
                  label='Disk (MB)'
                  required
                  min={editBounds.disk.min}
                  max={editBounds.disk.max}
                  value={editDisk}
                  onChange={(val) => setEditDisk(typeof val === 'number' ? val : editBounds.disk.min)}
                  description={boundsText('Max', editBounds.disk, formatBytes)}
                />
              </SimpleGrid>

              <Title order={5} mt='xs'>
                Feature Limits
              </Title>

              <SimpleGrid cols={{ base: 2, sm: 4 }} spacing='md'>
                <NumberInput
                  label='Allocations'
                  min={1}
                  max={editBounds.allocations}
                  value={editAllocations}
                  onChange={(val) => setEditAllocations(typeof val === 'number' ? val : 1)}
                  description={`Max: ${editBounds.allocations}`}
                />
                <NumberInput
                  label='Databases'
                  min={0}
                  max={editBounds.databases}
                  value={editDatabases}
                  onChange={(val) => setEditDatabases(typeof val === 'number' ? val : 0)}
                  description={`Max: ${editBounds.databases}`}
                />
                <NumberInput
                  label='Backups'
                  min={0}
                  max={editBounds.backups}
                  value={editBackups}
                  onChange={(val) => setEditBackups(typeof val === 'number' ? val : 0)}
                  description={`Max: ${editBounds.backups}`}
                />
                <NumberInput
                  label='Schedules'
                  min={0}
                  max={editBounds.schedules}
                  value={editSchedules}
                  onChange={(val) => setEditSchedules(typeof val === 'number' ? val : 0)}
                  description={`Max: ${editBounds.schedules}`}
                />
              </SimpleGrid>
            </>
          )}

          <ModalFooter>
            <Button variant='default' onClick={() => setIsEditOpen(false)}>
              Cancel
            </Button>
            <Button color='blue' loading={submitting} onClick={handleEditSubmit}>
              Save Changes
            </Button>
          </ModalFooter>
        </Stack>
      </Modal>

      {/* DELETE CONFIRMATION MODAL */}
      <ConfirmationModal
        opened={!!serverToDelete}
        onClose={() => setServerToDelete(null)}
        title='Delete Child Server'
        confirm='Delete Server'
        confirmColor='red'
        onConfirmed={handleDeleteConfirmed}
      >
        <Text size='sm'>
          Are you sure you want to permanently delete child server{' '}
          <Text span fw={700}>
            {serverToDelete?.name}
          </Text>
          ?
        </Text>
        <Text size='sm' c='red.4' mt='xs'>
          This will delete all server files and databases permanently. Reclaimed resources will be returned to the
          master server pool.
        </Text>
      </ConfirmationModal>
    </ServerContentContainer>
  );
}
