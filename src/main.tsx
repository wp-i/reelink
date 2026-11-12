import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'
import ResourceBrowser from './ResourceBrowser'
import './styles.css'

const resourceId = new URLSearchParams(window.location.search).get('resource')
const isResourceBrowser =
  resourceId !== null &&
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(resourceId)

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>{isResourceBrowser ? <ResourceBrowser /> : <App />}</React.StrictMode>,
)
