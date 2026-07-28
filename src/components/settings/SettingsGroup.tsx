import type { ReactNode } from "react";

// A labelled block of controls inside a section.
//
// Sections are the four things the page is about; groups are the parts within one. The `id`
// is what makes a group a deep-link target: a link that used to open a whole section now has
// to be able to land on the group inside it, or "add it in Settings" drops the reader at the
// top of a long card with no idea which field was meant.
export function SettingsGroup({
  id,
  title,
  status,
  children,
}: {
  /// Bare section name — rendered as `settings-<id>`, matching the scroll handler.
  id?: string;
  title: string;
  status?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="settings-group" id={id ? `settings-${id}` : undefined}>
      <header className="settings-group-header">
        <h3>{title}</h3>
        {status}
      </header>
      {children}
    </section>
  );
}
