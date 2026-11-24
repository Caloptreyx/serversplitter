import {
  faChevronDown,
  faChevronRight,
  faMagnifyingGlass,
  faRotate,
  faScaleBalanced,
} from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import {
  Badge,
  Card,
  Group,
  Pagination,
  Stack,
  Switch,
  Table,
  Text,
  TextInput,
  Title,
  UnstyledButton,
} from '@mantine/core';
import { useDebouncedValue } from '@mantine/hooks';
import { Fragment, useEffect, useState } from 'react';
import { httpErrorToHuman } from '@/api/axios.ts';
import AlertError from '@/elements/alerts/AlertError.tsx';
import Button from '@/elements/buttons/Button.tsx';
import TableLink from '@/elements/data-display/TableLink.tsx';
import Spinner from '@/elements/feedback/Spinner.tsx';
import ConfirmationModal from '@/elements/modals/ConfirmationModal.tsx';
import { bytesToString, mbToBytes } from '@/lib/format/size.ts';
import { useToast } from '@/providers/ToastProvider.tsx';
import {
  type AdminMasterRow,
  type AdminMastersPage,
  getAdminMasters,
  type PoolIssue,
  type ReconcileResult,
  reconcileAll,
  reconcileMaster,
} from '../api/admin.ts';
import { formatOvercommit } from './splitterFormat.ts';

const PER_PAGE = 25;
const COLUMNS = 6;

const issueKinds: Record<PoolIssue['kind'], string> = {
  drift: 'Limits changed',
  untracked: 'Not tracked',
  orphaned: 'Orphaned entry',
};

/** A limit where 0 means unlimited. CPU is a percentage, memory and disk are in MB. */
function formatAmount(value: number, unit: '%' | 'MB') {
  if (value === 0) return '∞';
  return unit === '%' ? `${value}%` : bytesToString(mbToBytes(value));
}

function PoolCell({ row }: { row: AdminMasterRow }) {
  const lines: { label: string; unit: '%' | 'MB'; held: number; own: number }[] = [
    { label: 'CPU', unit: '%', held: row.held.cpu, own: row.master.cpu },
    { label: 'Mem', unit: 'MB', held: row.held.memory, own: row.master.memory },
    { label: 'Disk', unit: 'MB', held: row.held.disk, own: row.master.disk },
  ];

  return (
    <div className='grid grid-cols-[auto_auto] gap-x-2 text-xs'>
      {lines.map((line) => (
        <Fragment key={line.label}>
          <span className='text-(--mantine-color-dimmed)'>{line.label}</span>
          <span className='whitespace-nowrap'>
            <b>{formatAmount(line.held, line.unit)}</b> / {formatAmount(line.own, line.unit)}
          </span>
        </Fragment>
      ))}
    </div>
  );
}

export default function AdminMastersTab({ canWrite }: { canWrite: boolean }) {
  const { addToast } = useToast();

  const [page, setPage] = useState(1);
  const [search, setSearch] = useState('');
  const [debouncedSearch] = useDebouncedValue(search, 300);
  const [onlyProblems, setOnlyProblems] = useState(false);

  const [data, setData] = useState<AdminMastersPage | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [reloadKey, setReloadKey] = useState(0);

  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [reconciling, setReconciling] = useState<string | null>(null);
  const [confirmAll, setConfirmAll] = useState(false);

  useEffect(() => {
    setPage(1);
  }, [debouncedSearch, onlyProblems]);

  useEffect(() => {
    let active = true;
    setLoading(true);
    setError('');
    getAdminMasters({
      page,
      per_page: PER_PAGE,
      search: debouncedSearch.trim() || undefined,
      status: onlyProblems ? 'drift' : undefined,
    })
      .then((result) => active && setData(result))
      .catch((err) => active && setError(httpErrorToHuman(err)))
      .finally(() => active && setLoading(false));
    return () => {
      active = false;
    };
  }, [page, debouncedSearch, onlyProblems, reloadKey]);

  const reportResult = (result: ReconcileResult) => {
    if (result.failed.length === 0) {
      addToast(`Reconciled ${result.fixed} split${result.fixed === 1 ? '' : 's'}.`, 'success');
      return;
    }
    const reasons = result.failed.map((failure) => `${failure.split_name}: ${failure.reason}`).join('; ');
    addToast(`Reconciled ${result.fixed}, failed ${result.failed.length}. ${reasons}`, 'warning');
  };

  const handleReconcile = async (row: AdminMasterRow) => {
    try {
      setReconciling(row.uuid);
      reportResult(await reconcileMaster(row.uuid));
      setReloadKey((key) => key + 1);
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setReconciling(null);
    }
  };

  const handleReconcileAll = async () => {
    reportResult(await reconcileAll());
    setConfirmAll(false);
    setReloadKey((key) => key + 1);
  };

  const toggleExpanded = (uuid: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (!next.delete(uuid)) next.add(uuid);
      return next;
    });

  const totalPages = data ? Math.max(1, Math.ceil(data.total / data.per_page)) : 1;

  return (
    <Card padding='lg' radius='md' withBorder>
      <Stack gap='md'>
        <Group justify='space-between' align='flex-start'>
          <div className='min-w-0 flex-1'>
            <Title order={4} fw={600}>
              Masters
            </Title>
            <Text size='xs' c='dimmed'>
              Drift means a split&apos;s limits were changed outside the splitter; Reconcile adopts the current limits
              and moves the difference to or from the master. Over-committed means the master&apos;s plan is smaller
              than what its splits hold plus the reserve.
            </Text>
          </div>
          {canWrite && (
            <Button
              variant='default'
              leftSection={<FontAwesomeIcon icon={faScaleBalanced} />}
              onClick={() => setConfirmAll(true)}
            >
              Reconcile all
            </Button>
          )}
        </Group>

        <Group gap='md' align='center'>
          <TextInput
            className='flex-1 min-w-48'
            placeholder='Search masters, owners or nodes'
            leftSection={<FontAwesomeIcon icon={faMagnifyingGlass} />}
            value={search}
            onChange={(e) => setSearch(e.currentTarget.value)}
          />
          <Switch
            label='Only with problems'
            checked={onlyProblems}
            onChange={(e) => setOnlyProblems(e.currentTarget.checked)}
          />
        </Group>

        {error && <AlertError error={error} setError={setError} />}

        {!data && loading ? (
          <div className='flex justify-center py-8'>
            <Spinner />
          </div>
        ) : data && data.masters.length === 0 ? (
          <div className='py-8 text-center'>
            <Text c='dimmed' size='sm'>
              {onlyProblems
                ? 'No master needs attention.'
                : debouncedSearch
                  ? 'No master matches your search.'
                  : 'No server has splits yet.'}
            </Text>
          </div>
        ) : (
          data && (
            <Table.ScrollContainer minWidth={760}>
              <Table verticalSpacing='sm' style={{ opacity: loading ? 0.6 : 1 }}>
                <Table.Thead>
                  <Table.Tr>
                    <Table.Th>Master</Table.Th>
                    <Table.Th>Owner</Table.Th>
                    <Table.Th>Splits</Table.Th>
                    <Table.Th>Pool (splits / master)</Table.Th>
                    <Table.Th>Status</Table.Th>
                    <Table.Th />
                  </Table.Tr>
                </Table.Thead>
                <Table.Tbody>
                  {data.masters.map((row) => {
                    const isOpen = expanded.has(row.uuid);
                    return (
                      <Fragment key={row.uuid}>
                        <Table.Tr>
                          <Table.Td>
                            <TableLink to={`/admin/servers/${row.uuid}`}>{row.name}</TableLink>
                            <Text size='xs' c='dimmed'>
                              {row.node.name}
                            </Text>
                          </Table.Td>
                          <Table.Td>
                            <TableLink to={`/admin/users/${row.owner.uuid}`}>{row.owner.username}</TableLink>
                          </Table.Td>
                          <Table.Td className='whitespace-nowrap'>
                            {row.splits_used} / {row.splits_limit}
                          </Table.Td>
                          <Table.Td>
                            <PoolCell row={row} />
                          </Table.Td>
                          <Table.Td>
                            {row.status === 'balanced' ? (
                              <Badge color='green' variant='light'>
                                Balanced
                              </Badge>
                            ) : (
                              <UnstyledButton onClick={() => toggleExpanded(row.uuid)} aria-expanded={isOpen}>
                                <Badge
                                  color={row.status === 'overcommitted' ? 'red' : 'orange'}
                                  variant='light'
                                  rightSection={<FontAwesomeIcon icon={isOpen ? faChevronDown : faChevronRight} />}
                                  className='cursor-pointer'
                                >
                                  {row.status === 'overcommitted'
                                    ? 'Over-committed'
                                    : `Needs attention (${row.issues.length})`}
                                </Badge>
                              </UnstyledButton>
                            )}
                          </Table.Td>
                          <Table.Td>
                            {canWrite && row.issues.length > 0 && (
                              <Button
                                size='xs'
                                variant='outline'
                                color='orange'
                                leftSection={<FontAwesomeIcon icon={faRotate} />}
                                loading={reconciling === row.uuid}
                                disabled={reconciling !== null && reconciling !== row.uuid}
                                onClick={() => handleReconcile(row)}
                              >
                                Reconcile
                              </Button>
                            )}
                          </Table.Td>
                        </Table.Tr>
                        {isOpen && row.status !== 'balanced' && (
                          <Table.Tr>
                            <Table.Td colSpan={COLUMNS} className='bg-(--mantine-color-default-hover)'>
                              <Stack gap={4}>
                                {row.overcommit && (
                                  <Text size='xs' c='red'>
                                    <b>Over-committed</b> · the plan is smaller than what the splits hold plus the
                                    reserve. Free {formatOvercommit(row.overcommit)} by shrinking or deleting splits.
                                  </Text>
                                )}
                                {row.issues.map((issue) => (
                                  <Text key={`${issue.split_uuid}-${issue.kind}`} size='xs'>
                                    <TableLink to={`/admin/servers/${issue.split_uuid}`}>{issue.split_name}</TableLink>
                                    {' · '}
                                    <b>{issueKinds[issue.kind]}</b>
                                    {issue.detail && (
                                      <span className='text-(--mantine-color-dimmed)'> · {issue.detail}</span>
                                    )}
                                  </Text>
                                ))}
                              </Stack>
                            </Table.Td>
                          </Table.Tr>
                        )}
                      </Fragment>
                    );
                  })}
                </Table.Tbody>
              </Table>
            </Table.ScrollContainer>
          )
        )}

        {data && totalPages > 1 && (
          <Group justify='flex-end'>
            <Pagination total={totalPages} value={page} onChange={setPage} size='sm' />
          </Group>
        )}
      </Stack>

      <ConfirmationModal
        opened={confirmAll}
        onClose={() => setConfirmAll(false)}
        title='Reconcile all masters'
        confirm='Reconcile all'
        confirmColor='orange'
        onConfirmed={handleReconcileAll}
      >
        <Text size='sm'>
          Every split whose limits were changed outside the splitter keeps its current limits, and the difference is
          taken from or returned to its master. Splits whose master can&apos;t cover the difference are left as they are
          and reported.
        </Text>
      </ConfirmationModal>
    </Card>
  );
}
