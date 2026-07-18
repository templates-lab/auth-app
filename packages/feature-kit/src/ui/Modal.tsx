import { createEffect, onCleanup, Show, type Component, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { X } from "lucide-solid";
import "./ui.css";

/**
 * Minimal modal dialog: fixed overlay, centered scrollable card, closes on
 * Escape, overlay click, or the header button. Rendered through a portal so
 * host stacking contexts never clip it.
 */
export const Modal: Component<{
  open: boolean;
  title: string;
  onClose: () => void;
  children: JSX.Element;
}> = (props) => {
  createEffect(() => {
    if (!props.open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") props.onClose();
    };
    document.addEventListener("keydown", onKey);
    onCleanup(() => document.removeEventListener("keydown", onKey));
  });

  return (
    <Show when={props.open}>
      <Portal>
        <div
          class="fk-modal__overlay"
          onClick={(e) => {
            if (e.target === e.currentTarget) props.onClose();
          }}
        >
          <div class="fk-modal" role="dialog" aria-modal="true" aria-label={props.title}>
            <header class="fk-modal__header">
              <h2 class="fk-modal__title">{props.title}</h2>
              <button
                type="button"
                class="fk-btn fk-btn--sm"
                aria-label="Cerrar"
                onClick={() => props.onClose()}
              >
                <X size={16} />
              </button>
            </header>
            <div class="fk-modal__body">{props.children}</div>
          </div>
        </div>
      </Portal>
    </Show>
  );
};
