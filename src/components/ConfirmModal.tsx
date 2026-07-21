import {
  useEffect,
  useId,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';
import {
  CircleX,
  Info,
  LoaderCircle,
  RotateCw,
  TriangleAlert,
  X,
} from 'lucide-react';

export type ConfirmModalTone =
  | 'default'
  | 'warning'
  | 'danger';

interface ConfirmModalProps {
  open: boolean;
  title: string;
  description: ReactNode;
  warning?: ReactNode;
  confirmLabel?: string;
  cancelLabel?: string;
  busyLabel?: string;
  tone?: ConfirmModalTone;
  busy?: boolean;
  icon?: ReactNode;
  closeOnBackdrop?: boolean;
  closeOnEscape?: boolean;
  autoFocus?: 'confirm' | 'cancel';
  onConfirm: () => void | Promise<void>;
  onCancel: () => void;
}

const EXIT_ANIMATION_MS = 160;

const FOCUSABLE_SELECTOR = [
  'button:not([disabled])',
  'a[href]',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  'not([tabindex="-1"])',
].join(',');

function ToneIcon({
  tone,
}: {
  tone: ConfirmModalTone;
}) {
  if (tone === 'warning') {
    return (
      <RotateCw
        size={24}
        strokeWidth={1.8}
      />
    );
  }

  if (tone === 'danger') {
    return (
      <CircleX
        size={24}
        strokeWidth={1.8}
      />
    );
  }

  return (
    <Info
      size={24}
      strokeWidth={1.8}
    />
  );
}

export function ConfirmModal({
  open,
  title,
  description,
  warning,
  confirmLabel = 'CONFIRM',
  cancelLabel = 'CANCEL',
  busyLabel = 'PLEASE WAIT...',
  tone = 'default',
  busy = false,
  icon,
  closeOnBackdrop = true,
  closeOnEscape = true,
  autoFocus = 'cancel',
  onConfirm,
  onCancel,
}: ConfirmModalProps) {
  const titleId = useId();
  const descriptionId = useId();

  const [mounted, setMounted] =
    useState(open);

  const [exiting, setExiting] =
    useState(false);

  const dialogRef =
    useRef<HTMLElement | null>(null);

  const confirmButtonRef =
    useRef<HTMLButtonElement | null>(null);

  const cancelButtonRef =
    useRef<HTMLButtonElement | null>(null);

  const previousFocusRef =
    useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (open) {
      setMounted(true);
      setExiting(false);
      return;
    }

    if (!mounted) {
      return;
    }

    setExiting(true);

    const exitTimer = window.setTimeout(() => {
      setMounted(false);
      setExiting(false);
    }, EXIT_ANIMATION_MS);

    return () => {
      window.clearTimeout(exitTimer);
    };
  }, [open, mounted]);

  useEffect(() => {
    if (!mounted || exiting) {
      return;
    }

    previousFocusRef.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;

    const previousOverflow =
      document.body.style.overflow;

    document.body.style.overflow = 'hidden';

    const focusFrame =
      window.requestAnimationFrame(() => {
        if (autoFocus === 'confirm') {
          confirmButtonRef.current?.focus();
        } else {
          cancelButtonRef.current?.focus();
        }
      });

    function handleDocumentKeyDown(
      event: globalThis.KeyboardEvent
    ) {
      if (
        event.key === 'Escape' &&
        closeOnEscape &&
        !busy
      ) {
        event.preventDefault();
        event.stopPropagation();
        onCancel();
      }
    }

    document.addEventListener(
      'keydown',
      handleDocumentKeyDown,
      true
    );

    return () => {
      window.cancelAnimationFrame(focusFrame);

      document.removeEventListener(
        'keydown',
        handleDocumentKeyDown,
        true
      );

      document.body.style.overflow =
        previousOverflow;

      window.requestAnimationFrame(() => {
        previousFocusRef.current?.focus();
      });
    };
  }, [
    mounted,
    exiting,
    autoFocus,
    busy,
    closeOnEscape,
    onCancel,
  ]);

  function handleFocusTrap(
    event: ReactKeyboardEvent<HTMLElement>
  ) {
    if (event.key !== 'Tab') {
      return;
    }

    const dialog = dialogRef.current;

    if (!dialog) {
      return;
    }

    const focusableElements = Array.from(
      dialog.querySelectorAll<HTMLElement>(
        FOCUSABLE_SELECTOR
      )
    ).filter((element) => {
      return (
        !element.hasAttribute('disabled') &&
        element.getAttribute('aria-hidden') !==
          'true' &&
        element.offsetParent !== null
      );
    });

    if (focusableElements.length === 0) {
      event.preventDefault();
      dialog.focus();
      return;
    }

    const firstElement =
      focusableElements[0];

    const lastElement =
      focusableElements[
        focusableElements.length - 1
      ];

    if (
      event.shiftKey &&
      document.activeElement === firstElement
    ) {
      event.preventDefault();
      lastElement.focus();
      return;
    }

    if (
      !event.shiftKey &&
      document.activeElement === lastElement
    ) {
      event.preventDefault();
      firstElement.focus();
    }
  }

  async function handleConfirm() {
    if (busy) {
      return;
    }

    await onConfirm();
  }

  if (!mounted) {
    return null;
  }

  const modal = (
    <div
      className={[
        'confirm-modal-backdrop',
        exiting
          ? 'confirm-modal-backdrop--exiting'
          : '',
      ]
        .filter(Boolean)
        .join(' ')}
      role="presentation"
      onMouseDown={(event) => {
        if (
          event.target === event.currentTarget &&
          closeOnBackdrop &&
          !busy
        ) {
          onCancel();
        }
      }}
    >
      <section
        ref={dialogRef}
        className={[
          'confirm-modal',
          `confirm-modal--${tone}`,
          busy
            ? 'confirm-modal--busy'
            : '',
          exiting
            ? 'confirm-modal--exiting'
            : '',
        ]
          .filter(Boolean)
          .join(' ')}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={descriptionId}
        aria-busy={busy}
        tabIndex={-1}
        onKeyDown={handleFocusTrap}
        onMouseDown={(event) => {
          event.stopPropagation();
        }}
      >
        <div
          className="confirm-modal-accent"
          aria-hidden="true"
        />

        <button
          type="button"
          className="confirm-modal-close"
          aria-label="Close confirmation"
          disabled={busy}
          onClick={onCancel}
        >
          <X
            size={15}
            strokeWidth={1.8}
            aria-hidden="true"
          />
        </button>

        <div
          className="confirm-modal-icon"
          aria-hidden="true"
        >
          {icon ?? <ToneIcon tone={tone} />}
        </div>

        <div className="confirm-modal-content">
          <h3
            id={titleId}
            className="confirm-modal-title"
          >
            {title}
          </h3>

          <div
            id={descriptionId}
            className="confirm-modal-description"
          >
            {description}
          </div>

          {warning && (
            <div className="confirm-modal-warning">
              <TriangleAlert
                className="confirm-modal-warning-icon"
                size={15}
                strokeWidth={1.9}
                aria-hidden="true"
              />

              <span className="confirm-modal-warning-text">
                {warning}
              </span>
            </div>
          )}
        </div>

        <div className="confirm-modal-actions">
          <button
            ref={cancelButtonRef}
            type="button"
            className="confirm-modal-button confirm-modal-button--secondary"
            disabled={busy}
            onClick={onCancel}
          >
            {cancelLabel}
          </button>

          <button
            ref={confirmButtonRef}
            type="button"
            className="confirm-modal-button confirm-modal-button--primary"
            disabled={busy}
            aria-busy={busy}
            onClick={() => {
              void handleConfirm();
            }}
          >
            {busy && (
              <LoaderCircle
                className="confirm-modal-spinner"
                size={15}
                strokeWidth={2}
                aria-hidden="true"
              />
            )}

            <span>
              {busy
                ? busyLabel
                : confirmLabel}
            </span>
          </button>
        </div>
      </section>
    </div>
  );

  return createPortal(
    modal,
    document.body
  );
}