/** What the placeholder card says for each reason a widget cannot run. */
export function unavailableText(reason: string): { title: string; hint: string } {
  switch (reason) {
    case 'unsupported':
      return { title: 'Custom widgets are not yet supported on Windows', hint: 'This widget comes from an extension pack. It will run here once Windows support lands.' };
    case 'browser':
      return { title: 'Custom widget', hint: 'Custom widgets run only in the desktop app.' };
    case 'locked':
      return { title: 'Custom widget', hint: 'Unlock the Hub to show this widget.' };
    default:
      return { title: 'Custom widget unavailable', hint: 'Its pack is not installed, is disabled, or is not trusted to run code.' };
  }
}
