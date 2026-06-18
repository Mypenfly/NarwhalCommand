// ts_component.tsx — React TypeScript 组件
//
// 真实工程风格的 React 组件代码，
// 包含 Props 接口、hooks、条件渲染、JSX 模板，
// 用于验证 NCS 对 TypeScript/JSX 语法的编辑能力。

import React, { useState, useCallback, useEffect } from 'react';

export interface ButtonProps {
  label: string;
  variant?: 'primary' | 'secondary' | 'danger';
  size?: 'sm' | 'md' | 'lg';
  disabled?: boolean;
  loading?: boolean;
  onClick?: () => void;
}

interface ButtonState {
  isHovered: boolean;
  isPressed: boolean;
}

const variantStyles: Record<string, React.CSSProperties> = {
  primary: {
    backgroundColor: '#2563eb',
    color: '#ffffff',
    border: 'none',
  },
  secondary: {
    backgroundColor: '#f3f4f6',
    color: '#1f2937',
    border: '1px solid #d1d5db',
  },
  danger: {
    backgroundColor: '#dc2626',
    color: '#ffffff',
    border: 'none',
  },
};

const sizeStyles: Record<string, React.CSSProperties> = {
  sm: { padding: '4px 12px', fontSize: '0.875rem' },
  md: { padding: '8px 16px', fontSize: '1rem' },
  lg: { padding: '12px 24px', fontSize: '1.125rem' },
};

export const Button: React.FC<ButtonProps> = ({
  label,
  variant = 'primary',
  size = 'md',
  disabled = false,
  loading = false,
  onClick,
}) => {
  const [state, setState] = useState<ButtonState>({
    isHovered: false,
    isPressed: false,
  });

  useEffect(() => {
    return () => {
      setState({ isHovered: false, isPressed: false });
    };
  }, []);

  const handleClick = useCallback(() => {
    if (!disabled && !loading && onClick) {
      onClick();
    }
  }, [disabled, loading, onClick]);

  const handleMouseEnter = () => {
    setState(prev => ({ ...prev, isHovered: true }));
  };

  const handleMouseLeave = () => {
    setState(prev => ({ ...prev, isHovered: false, isPressed: false }));
  };

  const containerStyle: React.CSSProperties = {
    cursor: disabled || loading ? 'not-allowed' : 'pointer',
    opacity: disabled ? 0.5 : loading ? 0.7 : 1,
    borderRadius: '6px',
    fontWeight: 600,
    display: 'inline-flex',
    alignItems: 'center',
    gap: '8px',
    ...variantStyles[variant],
    ...sizeStyles[size],
  };

  return (
    <button
      style={containerStyle}
      disabled={disabled || loading}
      onClick={handleClick}
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
    >
      {loading && <Spinner />}
      {label}
    </button>
  );
};

const Spinner: React.FC = () => (
  <span className="spinner" style={{ animation: 'spin 1s linear infinite' }}>
    ⟳
  </span>
);

export default Button;
