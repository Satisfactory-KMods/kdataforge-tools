import { DocsIndex, DocsSearch, docsRef } from '../docsIndex';

export interface LocalDocsHit { ref: string; title: string; heading: string; snippet: string; score: number }

/** Bundled index only. No network client or generated routes. */
export function searchBundledDocs(index: DocsIndex, query: string, limit = 10): LocalDocsHit[] {
  if (typeof query !== 'string' || query.length > 256 || !Number.isInteger(limit) || limit < 0) return [];
  const count = Math.min(limit, 20);
  if (!query.trim() || !count) return [];
  const pages = new Map(index.pages.filter(page => !/(^|\/)(?:retired|legacy|standalone|installer|updater)(?:\/|$)/i.test(page.path))
    .map(page => [page.path, page]));
  return new DocsSearch({ ...index, pages: [...pages.values()] }).search(query, count).flatMap(hit => {
    const ref = docsRef(hit.ref);
    const [path, anchor = ''] = ref.split('#');
    const page = pages.get(path);
    const section = page?.sections.find(item => item.anchor === anchor);
    return page && section ? [{ ref, title: page.title, heading: section.heading,
      snippet: section.text.slice(0, 240), score: hit.score }] : [];
  });
}
