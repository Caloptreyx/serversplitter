import { faCircleInfo } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { useEffect, useState } from 'react';
import { useParams } from 'react-router';
import TableLink from '@/elements/data-display/TableLink.tsx';
import Alert from '@/elements/feedback/Alert.tsx';
import { getAdminServerSplits, type ParentServer } from '../api/client.ts';

/**
 * Notice at the top of every admin page of a split server. The container slot it lives in is
 * shared with the admin servers list, which has no server id, so it renders nothing there.
 */
export default function AdminSplitNotice() {
  const { id } = useParams<'id'>();
  const [parent, setParent] = useState<ParentServer | null>(null);

  useEffect(() => {
    setParent(null);
    if (!id) return;

    let active = true;
    getAdminServerSplits(id)
      .then((splits) => active && setParent(splits.parent))
      // the Overview card reports load failures; here the notice just stays hidden
      .catch(() => active && setParent(null));
    return () => {
      active = false;
    };
  }, [id]);

  if (!parent) return null;

  return (
    <Alert title='Split server' color='orange' icon={<FontAwesomeIcon icon={faCircleInfo} />} mb='md'>
      This server is a split of <TableLink to={`/admin/servers/${parent.uuid}`}>{parent.name}</TableLink>. Its resources
      come out of that server&apos;s pool, so change them from the master&apos;s Splitter page rather than here.
    </Alert>
  );
}
