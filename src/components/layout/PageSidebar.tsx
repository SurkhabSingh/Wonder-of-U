import type { PageNavigationItem } from "../../lib/navigation";
import { isSetupPage } from "../../lib/navigation";
import type { AppPage } from "../../types";

function PageLink({
  page,
  active,
  onSelect,
}: {
  page: PageNavigationItem;
  active: boolean;
  onSelect: (page: AppPage) => void;
}) {
  return (
    <button
      type="button"
      className={`page-link ${active ? "active" : ""}`}
      onClick={() => onSelect(page.id)}
    >
      <span className="page-link-label">
        {page.label}
        {page.count ? (
          <strong className="status-chip-count">{page.count}</strong>
        ) : null}
      </span>
      {page.description ? <small>{page.description}</small> : null}
    </button>
  );
}

export function PageSidebar({
  activePage,
  workflowPages,
  setupEntry,
  onPageSelect,
}: {
  activePage: AppPage;
  workflowPages: PageNavigationItem[];
  setupEntry: PageNavigationItem;
  onPageSelect: (page: AppPage) => void;
}) {
  return (
    <aside className="page-sidebar" aria-label="Application sections">
      <div className="sidebar-title">
        <strong>Wonder of U</strong>
      </div>
      <nav className="page-primary-nav" aria-label="Primary">
        {workflowPages.map((page) => (
          <PageLink
            key={page.id}
            page={page}
            active={activePage === page.id}
            onSelect={onPageSelect}
          />
        ))}
      </nav>
      <nav className="page-setup-nav" aria-label="Setup">
        <PageLink
          page={setupEntry}
          active={isSetupPage(activePage)}
          onSelect={onPageSelect}
        />
      </nav>
    </aside>
  );
}
