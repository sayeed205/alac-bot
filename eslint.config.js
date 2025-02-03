import antfu from '@antfu/eslint-config'

export default antfu({
    stylistic: {
        indent: 4,
    },
    typescript: true,
    yaml: false,
    rules: {
        'curly': ['error', 'multi-line'],
        'style/brace-style': ['error', '1tbs', { allowSingleLine: true }],
        'style/quotes': ['error', 'single', { avoidEscape: true }],
        'import/order': ['error', { 'newlines-between': 'always' }],
        'antfu/if-newline': 'off',
        'style/max-statements-per-line': ['error', { max: 2 }],
        'no-console': 'off',
        'antfu/no-top-level-await': 'off',
    },
})
