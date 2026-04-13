#!/bin/bash
# Notesdaq crawl progress dashboard
# Usage: ./stats.sh

DB="docker exec notesdaq-db-1 psql -U notesdaq -d notesdaq --no-align -t"

echo "═══════════════════════════════════════════"
echo "  NOTESDAQ CRAWL STATUS  $(date +%H:%M:%S)"
echo "═══════════════════════════════════════════"
echo

# Overall counts
read reblogs likes snapshots enumerated total_posts blogs <<< $(
  $DB -c "
    SELECT
      (SELECT count(*) FROM reblog_events),
      (SELECT count(*) FROM likes),
      (SELECT count(*) FROM reblog_count_snapshots),
      (SELECT count(*) FROM posts WHERE notes_fully_enumerated),
      (SELECT count(*) FROM posts),
      (SELECT count(*) FROM blogs)
  " | tr '|' ' '
)

printf "  Posts:       %'d\n" "$total_posts"
printf "  Blogs:       %'d\n" "$blogs"
printf "  Enumerated:  %'d / %'d posts  (%.1f%%)\n" "$enumerated" "$total_posts" \
  "$(echo "scale=1; $enumerated * 100 / $total_posts" | bc 2>/dev/null || echo 0)"
printf "  Reblogs:     %'d\n" "$reblogs"
printf "  Likes:       %'d\n" "$likes"
printf "  Snapshots:   %'d\n" "$snapshots"
echo

# Crawl queue
echo "── Crawl Queue ─────────────────────────────"
$DB -c "
  SELECT status, count(*) FROM crawl_queue GROUP BY status ORDER BY status
" | while IFS='|' read status count; do
  printf "  %-12s %'d\n" "$status" "$count"
done
echo

# Current activity (last 5 log lines with timestamps)
echo "── Recent Log ──────────────────────────────"
tail -5 /tmp/fetcher.log 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g' | sed 's/^/  /'
echo

# Top 5 posts by reblog count
echo "── Top Stocks (by reblogs) ─────────────────"
$DB -c "
  SELECT
    s.post_id,
    s.reblog_count as price,
    b.slug as op,
    left(p.summary, 40) as summary
  FROM reblog_count_snapshots s
  JOIN posts p ON p.post_id = s.post_id
  JOIN blogs b ON b.uuid = p.blog_uuid
  ORDER BY s.reblog_count DESC
  LIMIT 5
" | while IFS='|' read post_id price op summary; do
  [ -z "$post_id" ] && continue
  printf "  %s  price: %'d  op: %s  %s\n" "$post_id" "$price" "$op" "$summary"
done
echo

# 5 most recent reblog events
echo "── Recent Trades ───────────────────────────"
$DB -c "
  SELECT
    b.slug,
    re.reblog_count_at_time as price,
    re.post_id,
    to_char(re.reblogged_at, 'YYYY-MM-DD HH24:MI') as ts
  FROM reblog_events re
  JOIN blogs b ON b.uuid = re.blog_uuid
  WHERE NOT re.is_op_position
  ORDER BY re.created_in_db_at DESC
  LIMIT 5
" | while IFS='|' read slug price post_id ts; do
  [ -z "$slug" ] && continue
  printf "  %s bought %s at price %'d  (%s)\n" "$slug" "$post_id" "$price" "$ts"
done
echo

# Fetcher process check
if pgrep -f "target/debug/notesdaq-fetch" > /dev/null; then
  echo "  ✓ Fetcher is running"
else
  echo "  ✗ Fetcher is NOT running"
fi
