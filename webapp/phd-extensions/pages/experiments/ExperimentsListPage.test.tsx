import { afterEach, describe, expect, it } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import type { AxiosInstance } from 'axios';
import ExperimentsListPage from './ExperimentsListPage';
import { setExperimentsClient } from '../../lib/experiments/api';

function mockClient(payload: unknown): AxiosInstance {
  return {
    get: () => Promise.resolve({ data: payload }),
  } as unknown as AxiosInstance;
}

afterEach(() => setExperimentsClient(null));

function renderPage() {
  return render(
    <MemoryRouter>
      <ExperimentsListPage />
    </MemoryRouter>,
  );
}

describe('<ExperimentsListPage />', () => {
  it('shows a helpful empty state when no experiments exist', async () => {
    setExperimentsClient(mockClient({ experiments: [] }));
    renderPage();
    expect(await screen.findByText(/no experiments yet/i)).toBeInTheDocument();
    expect(screen.getAllByText(/new experiment/i).length).toBeGreaterThan(0);
  });

  it('renders one card per experiment with status pill and counters', async () => {
    setExperimentsClient(
      mockClient({
        experiments: [
          {
            experiment_slug: 'baseline',
            run_id: 'run-1',
            experiment_name: 'baseline',
            output_dir: '/tmp',
            created_at: '2024-01-01T00:00:00Z',
            updated_at: '2024-01-02T00:00:00Z',
            total_cells: 10,
            completed_cells: 7,
            failed_cells: 1,
            running_cells: 2,
            status: 'running',
          },
        ],
      }),
    );
    renderPage();
    await waitFor(() => expect(screen.getByText('baseline')).toBeInTheDocument());
    expect(screen.getByText('run-1')).toBeInTheDocument();
    // Counters: total=10, done=7, failed=1
    expect(screen.getByText('10')).toBeInTheDocument();
    expect(screen.getByText('7')).toBeInTheDocument();
    expect(screen.getByText('1')).toBeInTheDocument();
    expect(screen.getAllByText(/running/i).length).toBeGreaterThan(0);
  });

  it('surfaces fetch errors with a retry control', async () => {
    setExperimentsClient({
      get: () => Promise.reject(new Error('network down')),
    } as unknown as AxiosInstance);
    renderPage();
    expect(await screen.findByText(/network down/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /retry/i })).toBeInTheDocument();
  });
});
