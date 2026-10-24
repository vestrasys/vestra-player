export const i18n = (scope) => Object.freeze({
    get locale() {
        return scope.runtime.locale;
    },
});
