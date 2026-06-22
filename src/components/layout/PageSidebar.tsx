import * as AccordionPrimitive from "@radix-ui/react-accordion";
import type { PageNavigationItem } from "../../lib/navigation";
import type { AppPage } from "../../types";

function PageLink({
  page,
  activePage,
  onSelect,
}: {
  page: PageNavigationItem;
  activePage: AppPage;
  onSelect: (page: AppPage) => void;
}) {
  return (
    <button
      type="button"
      className={`page-link ${activePage === page.id ? "active" : ""}`}
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
  setupPages,
  onPageSelect,
}: {
  activePage: AppPage;
  activePageLabel: string;
  workflowPages: PageNavigationItem[];
  setupPages: PageNavigationItem[];
  onPageSelect: (page: AppPage) => void;
}) {
  return (
    <aside className="page-sidebar" aria-label="Application sections">
      <div className="sidebar-title">
        <p className="panel-kicker">Wonder of U</p>
        <strong>{activePageLabel}</strong>
      </div>
      <AccordionPrimitive.Root
        type="multiple"
        defaultValue={["workflow", "setup"]}
        className="page-accordion"
      >
        <AccordionPrimitive.Item value="workflow" className="page-accordion-item">
          <AccordionPrimitive.Header>
            <AccordionPrimitive.Trigger className="page-accordion-trigger">
              Workflow
              <span aria-hidden="true">{"\u2304"}</span>
            </AccordionPrimitive.Trigger>
          </AccordionPrimitive.Header>
          <AccordionPrimitive.Content className="page-accordion-content">
            {workflowPages.map((page) => (
              <PageLink
                key={page.id}
                page={page}
                activePage={activePage}
                onSelect={onPageSelect}
              />
            ))}
          </AccordionPrimitive.Content>
        </AccordionPrimitive.Item>

        <AccordionPrimitive.Item value="setup" className="page-accordion-item">
          <AccordionPrimitive.Header>
            <AccordionPrimitive.Trigger className="page-accordion-trigger">
              Setup
              <span aria-hidden="true">{"\u2304"}</span>
            </AccordionPrimitive.Trigger>
          </AccordionPrimitive.Header>
          <AccordionPrimitive.Content className="page-accordion-content">
            {setupPages.map((page) => (
              <PageLink
                key={page.id}
                page={page}
                activePage={activePage}
                onSelect={onPageSelect}
              />
            ))}
          </AccordionPrimitive.Content>
        </AccordionPrimitive.Item>
      </AccordionPrimitive.Root>
    </aside>
  );
}
