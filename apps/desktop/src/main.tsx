import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import { AppearanceProvider } from './appearance';
import './styles.css';

ReactDOM.createRoot(document.getElementById('root')!).render(<React.StrictMode><AppearanceProvider><App /></AppearanceProvider></React.StrictMode>);
