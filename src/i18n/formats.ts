import type { IntlDateTimeFormats, IntlNumberFormats } from 'vue-i18n';

/**
 * Named formats for `d()` and `n()`. Every locale carries the same names so a
 * call site never has to know which one is active.
 */
export const datetimeFormats: IntlDateTimeFormats = {
  en: {
    time: { hour: '2-digit', minute: '2-digit', hour12: false },
    weekday: { weekday: 'short' },
    date: { day: 'numeric', month: 'short' },
    dateWithYear: { day: 'numeric', month: 'short', year: 'numeric' },
    dateTime: { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit', hour12: false },
    long: { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' },
  },
  de: {
    time: { hour: '2-digit', minute: '2-digit', hour12: false },
    weekday: { weekday: 'short' },
    date: { day: 'numeric', month: 'short' },
    dateWithYear: { day: 'numeric', month: 'short', year: 'numeric' },
    dateTime: { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit', hour12: false },
    long: { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' },
  },
};

export const numberFormats: IntlNumberFormats = {
  en: {
    integer: { maximumFractionDigits: 0 },
    decimal: { minimumFractionDigits: 1, maximumFractionDigits: 1 },
    percent: { style: 'percent', maximumFractionDigits: 0 },
  },
  de: {
    integer: { maximumFractionDigits: 0 },
    decimal: { minimumFractionDigits: 1, maximumFractionDigits: 1 },
    percent: { style: 'percent', maximumFractionDigits: 0 },
  },
};
