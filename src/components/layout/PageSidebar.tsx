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
      <span>{page.label}</span>
      <small>{page.description}</small>
    </button>
  );
}

export function PageSidebar({
  activePage,
  activePageLabel,
  workflowPages,
  setupEntry,
  onPageSelect,
}: {
  activePage: AppPage;
  activePageLabel: string;
  workflowPages: PageNavigationItem[];
  setupEntry: PageNavigationItem;
  onPageSelect: (page: AppPage) => void;
}) {
  return (
    <aside className="page-sidebar" aria-label="Application sections">
      <div className="sidebar-title">
        <p className="panel-kicker">Wonder of U</p>
        <strong>{activePageLabel}</strong>
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
